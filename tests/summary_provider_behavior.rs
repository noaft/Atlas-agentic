use atlas_agentic::models::{
    deepseek::{DeepSeekConfig, DeepSeekProvider},
    ModelProvider, ModelRequest, SummaryRequest,
};
use std::{
    io::{BufRead, BufReader, Read, Write},
    net::TcpListener,
    sync::mpsc,
    thread,
};
fn server(
    body: serde_json::Value,
) -> (
    DeepSeekProvider,
    mpsc::Receiver<serde_json::Value>,
    thread::JoinHandle<()>,
) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let (tx, rx) = mpsc::channel();
    let handle = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut reader = BufReader::new(&mut stream);
        let mut len = 0;
        loop {
            let mut line = String::new();
            reader.read_line(&mut line).unwrap();
            if line == "\r\n" {
                break;
            }
            if let Some(n) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                len = n.trim().parse::<usize>().unwrap();
            }
        }
        let mut bytes = vec![0; len];
        reader.read_exact(&mut bytes).unwrap();
        tx.send(serde_json::from_slice(&bytes).unwrap()).unwrap();
        drop(reader);
        let body = body.to_string();
        write!(stream,"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len()).unwrap();
    });
    (
        DeepSeekProvider::new(DeepSeekConfig {
            api_key: "fake".into(),
            base_url: format!("http://{address}"),
            model: "selected".into(),
        }),
        rx,
        handle,
    )
}
#[test]
fn summary_uses_only_system_and_one_input_with_explicit_output_limit() {
    let (provider, rx, thread) = server(
        serde_json::json!({"choices":[{"message":{"content":"summary"},"finish_reason":"length"}],"usage":{"prompt_tokens":100,"completion_tokens":12,"total_tokens":112}}),
    );
    let response = provider
        .summarize(&SummaryRequest {
            system_prompt: "summary instructions",
            content: "historical data",
            max_output_tokens: 12,
        })
        .unwrap();
    assert_eq!(response.finish_reason, "length");
    assert_eq!(response.usage.unwrap().total_tokens, 112);
    let request = rx.recv().unwrap();
    assert_eq!(request["max_tokens"], 12);
    assert_eq!(request["stream"], false);
    assert!(request.get("tools").is_none());
    assert!(request.get("response_format").is_none());
    assert_eq!(request["messages"].as_array().unwrap().len(), 2);
    thread.join().unwrap();
}
#[test]
fn memory_blocks_are_separate_messages_and_budget_counts_the_rendered_prompt() {
    use atlas_agentic::conversations::MemoryBlock;
    let (provider, rx, thread) =
        server(serde_json::json!({"choices":[{"message":{"content":"answer"}}]}));
    let blocks = vec![
        MemoryBlock {
            start_index: 0,
            end_index_exclusive: 2,
            content: "fact one".into(),
        },
        MemoryBlock {
            start_index: 2,
            end_index_exclusive: 4,
            content: "fact two".into(),
        },
    ];
    let request = ModelRequest {
        memory_blocks: &blocks,
        max_output_tokens: Some(100),
        system_prompt: "system",
        user_prompt: "new",
        history: &[],
        activity_context: Some("prior activity"),
        tools: &[],
        images: &[],
        temperature: None,
        top_p: None,
    };
    let count = provider.estimate_request(&request).unwrap().unwrap();
    assert!(count.estimated);
    assert!(count.tokens > 100);
    provider.complete(&request).unwrap();
    let wire = rx.recv().unwrap();
    let messages = wire["messages"].as_array().unwrap();
    assert_eq!(messages.len(), 4);
    assert!(messages[1]["content"]
        .as_str()
        .unwrap()
        .contains("fact one"));
    assert!(messages[2]["content"]
        .as_str()
        .unwrap()
        .contains("fact two"));
    assert_eq!(messages[3]["content"], "new");
    thread.join().unwrap();
}
