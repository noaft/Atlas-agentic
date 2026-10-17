use atlas_agentic::models::{
    deepseek::{DeepSeekConfig, DeepSeekProvider},
    ModelProvider, ModelRequest,
};
use std::{
    io::{BufRead, BufReader, Read, Write},
    net::{TcpListener, TcpStream},
    sync::mpsc,
    thread,
};

fn read_request(socket: &mut TcpStream) -> serde_json::Value {
    let mut reader = BufReader::new(socket);
    let mut length = 0;
    loop {
        let mut line = String::new();
        reader.read_line(&mut line).unwrap();
        if line == "\r\n" {
            break;
        }
        if let Some(value) = line.to_ascii_lowercase().strip_prefix("content-length:") {
            length = value.trim().parse::<usize>().unwrap();
        }
    }
    let mut request = vec![0; length];
    reader.read_exact(&mut request).unwrap();
    serde_json::from_slice(&request).unwrap()
}

fn provider(
    body: String,
) -> (
    DeepSeekProvider,
    mpsc::Receiver<serde_json::Value>,
    thread::JoinHandle<()>,
) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let (tx, rx) = mpsc::channel();
    let server = thread::spawn(move || {
        let (mut socket, _) = listener.accept().unwrap();
        tx.send(read_request(&mut socket)).unwrap();
        write!(socket, "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len()).unwrap();
        for byte in body.bytes() {
            socket.write_all(&[byte]).unwrap();
        }
    });
    (
        DeepSeekProvider::new(DeepSeekConfig {
            api_key: "fake-key".into(),
            base_url: format!("http://{address}"),
            model: "test-model".into(),
        }),
        rx,
        server,
    )
}
fn request() -> ModelRequest<'static> {
    ModelRequest {
        system_prompt: "system instructions",
        user_prompt: "hello",
        history: &[],
        activity_context: None,
        tools: &[],
        images: &[],
        temperature: None,
        top_p: None,
    }
}

#[test]
fn streaming_usage_covers_the_whole_prompt_and_output_without_double_counting() {
    let body = concat!(
        "data: {\"choices\":[{\"delta\":{\"content\":\"Xin chào 🌍\"}}],\"usage\":null}\n\n",
        "data: {\"choices\":[],\"usage\":{\"prompt_tokens\":1200,\"completion_tokens\":300,\"total_tokens\":1500,\"prompt_cache_hit_tokens\":1000,\"completion_tokens_details\":{\"reasoning_tokens\":200}}}\n\n",
        "data: [DONE]\n"
    );
    let (provider, requests, server) = provider(body.into());
    let mut text = String::new();
    let response = provider
        .stream_with_usage(&request(), &mut |delta| text.push_str(delta))
        .unwrap();
    assert_eq!(text, "Xin chào 🌍");
    assert_eq!(response.content, text);
    let usage = response.usage.unwrap();
    assert_eq!(usage.model, "test-model");
    assert_eq!(
        (usage.input_tokens, usage.output_tokens, usage.total_tokens),
        (1200, 300, 1500)
    );
    let request = requests.recv().unwrap();
    assert_eq!(request["stream_options"]["include_usage"], true);
    assert_eq!(request["messages"][0]["content"], "system instructions");
    server.join().unwrap();
}

#[test]
fn terminal_usage_without_newline_is_read_and_missing_or_invalid_usage_stays_unknown() {
    for (raw, expected) in [
        (
            r#"{"prompt_tokens":20,"completion_tokens":5,"total_tokens":25}"#,
            Some(25),
        ),
        ("null", None),
        (
            r#"{"prompt_tokens":20,"completion_tokens":5,"total_tokens":999}"#,
            None,
        ),
        (r#"{"prompt_tokens":-1,"completion_tokens":5}"#, None),
    ] {
        let body = format!(
            "data: {{\"choices\":[{{\"delta\":{{\"content\":\"answer\"}}}}],\"usage\":{raw}}}"
        );
        let (provider, _requests, server) = provider(body);
        let response = provider.stream_with_usage(&request(), &mut |_| {}).unwrap();
        assert_eq!(response.content, "answer");
        assert_eq!(response.usage.map(|usage| usage.total_tokens), expected);
        server.join().unwrap();
    }
}

use atlas_agentic::{
    agents::{AgentRegistry, LocalAgent, LocalAgentDefinition},
    conversations::{ConversationService, ConversationStore},
    models::{ModelResponse, ModelUsage},
    tools::{PwdTool, ToolRegistry},
    Runtime,
};
use std::{cell::RefCell, collections::VecDeque};
struct UsageModel(RefCell<VecDeque<ModelResponse>>);
impl ModelProvider for UsageModel {
    fn complete(&self, _: &ModelRequest) -> Result<String, String> {
        Ok("Generated title".into())
    }
    fn stream(
        &self,
        request: &ModelRequest,
        delta: &mut dyn FnMut(&str),
    ) -> Result<String, String> {
        self.stream_with_usage(request, delta)
            .map(|response| response.content)
    }
    fn stream_with_usage(
        &self,
        _: &ModelRequest,
        delta: &mut dyn FnMut(&str),
    ) -> Result<ModelResponse, String> {
        let response = self.0.borrow_mut().pop_front().ok_or("no fake response")?;
        delta(&response.content);
        Ok(response)
    }
}
fn usage(total: u64) -> ModelUsage {
    ModelUsage {
        model: "test-model".into(),
        input_tokens: total - 5,
        output_tokens: 5,
        total_tokens: total,
    }
}
#[test]
fn latest_chat_usage_is_persisted_without_summing_tools_or_overwriting_with_title_usage() {
    let data = tempfile::tempdir().unwrap();
    let model = UsageModel(RefCell::new(VecDeque::from([
        ModelResponse {
            content: r#"{"type":"tool_call","tool":"pwd","args":{}}"#.into(),
            usage: Some(usage(120)),
        },
        ModelResponse {
            content: r#"{"type":"final","content":"done"}"#.into(),
            usage: Some(usage(240)),
        },
        ModelResponse {
            content: r#"{"type":"final","content":"next"}"#.into(),
            usage: None,
        },
        ModelResponse {
            content: r#"{"type":"tool_call","tool":"pwd","args":{}}"#.into(),
            usage: Some(usage(360)),
        },
    ])));
    let definition = LocalAgentDefinition::from_toml(
        r#"
name = "test"
display_name = "Test"
model_provider = "fake"
model = "test-model"
system_prompt = "System instructions"
conversation_title_prompt = "Title only"
"#,
    )
    .unwrap();
    let runtime = Runtime::new(AgentRegistry::new(vec![Box::new(LocalAgent::new(
        definition,
        Box::new(model),
    ))]))
    .with_tools(ToolRegistry::new(vec![Box::new(PwdTool::new(data.path()))]));
    let store = ConversationStore::at(data.path().join("history"));
    let service = ConversationService::new(&runtime, &store, 40);
    let first = service.ask("task", None, &mut |_| {}).unwrap();
    let loaded = store.load(&first.metadata.id).unwrap();
    let json = serde_json::to_value(&loaded.activities).unwrap();
    assert_eq!(json[0]["usage"]["totalTokens"], 240);
    assert_eq!(json[0]["usage"]["inputTokens"], 235);
    assert_eq!(json[0]["usage"]["model"], "test-model");
    assert_eq!(loaded.metadata.title, "Generated title");
    service
        .ask("next", Some(&first.metadata.id), &mut |_| {})
        .unwrap();
    let loaded = store.load(&first.metadata.id).unwrap();
    let json = serde_json::to_value(&loaded.activities).unwrap();
    assert!(json[1]["usage"].is_null());
    let before_failure = serde_json::json!([loaded.metadata, loaded.messages, loaded.activities]);
    assert!(service
        .ask("fails after a tool", Some(&first.metadata.id), &mut |_| {})
        .is_err());
    let after_failure = store.load(&first.metadata.id).unwrap();
    assert_eq!(
        serde_json::json!([
            after_failure.metadata,
            after_failure.messages,
            after_failure.activities
        ]),
        before_failure
    );
}

#[test]
fn interrupted_stream_does_not_return_a_successful_usage_reading() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let server = thread::spawn(move || {
        let (mut socket, _) = listener.accept().unwrap();
        let _request = read_request(&mut socket);
        let body = "data: {\"choices\":[{\"delta\":{\"content\":\"partial\"}}]}\n\n";
        write!(
            socket,
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len() + 100
        )
        .unwrap();
    });
    let provider = DeepSeekProvider::new(DeepSeekConfig {
        api_key: "fake-key".into(),
        base_url: format!("http://{address}"),
        model: "test-model".into(),
    });
    assert!(provider.stream_with_usage(&request(), &mut |_| {}).is_err());
    server.join().unwrap();
}
