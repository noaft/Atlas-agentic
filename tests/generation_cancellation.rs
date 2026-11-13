use atlas_agentic::{
    agents::LocalAgentDefinition,
    models::{provider_from_definition_with_settings, ModelRequest},
    settings::AtlasSettings,
};
use serde_json::json;
use std::{
    io::{BufRead, BufReader, Read, Write},
    net::TcpListener,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    thread,
    time::{Duration, Instant},
};
fn request() -> ModelRequest<'static> {
    ModelRequest {
        memory_blocks: &[],
        max_output_tokens: Some(500),
        system_prompt: "system",
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
fn interruption_closes_silent_inference_before_headers_or_next_token() {
    for provider in [
        "deepseek",
        "openai",
        "anthropic",
        "gemini",
        "openai-compatible",
    ] {
        for headers in [false, true] {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let url = format!("http://{}", listener.local_addr().unwrap());
            let cancel = Arc::new(AtomicBool::new(false));
            let signal = cancel.clone();
            let server = thread::spawn(move || {
                let (mut socket, _) = listener.accept().unwrap();
                socket
                    .set_read_timeout(Some(Duration::from_secs(2)))
                    .unwrap();
                let mut reader = BufReader::new(&mut socket);
                let mut length = 0;
                loop {
                    let mut line = String::new();
                    reader.read_line(&mut line).unwrap();
                    if line == "\r\n" {
                        break;
                    }
                    if let Some(value) = line.to_lowercase().strip_prefix("content-length:") {
                        length = value.trim().parse().unwrap();
                    }
                }
                reader.read_exact(&mut vec![0; length]).unwrap();
                drop(reader);
                if headers {
                    socket.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\n").unwrap();
                    socket.flush().unwrap();
                }
                thread::sleep(Duration::from_millis(20));
                signal.store(true, Ordering::Release);
                matches!(socket.read(&mut [0; 1]), Ok(0))
            });
            let mut settings = serde_json::to_value(AtlasSettings::default()).unwrap();
            settings["selectedProvider"] = json!(provider);
            settings["providers"] =
                json!({provider: {"apiKey": "test-secret", "baseUrl": url, "model": "alias"}});
            if provider == "deepseek" {
                settings["deepseek"] = settings["providers"][provider].clone();
            }
            let settings: AtlasSettings = serde_json::from_value(settings).unwrap();
            let effective = settings.resolve_with(|_| None).unwrap();
            let definition = LocalAgentDefinition::from_toml(&format!("name='test'\ndisplay_name='Test'\nmodel_provider='{provider}'\nmodel='alias'\nsystem_prompt='System'\nconversation_title_prompt='Title'\n")).unwrap();
            let model = provider_from_definition_with_settings(&definition, &effective).unwrap();
            let started = Instant::now();
            let mut output = String::new();
            let result =
                model.stream_with_control(&request(), &mut |text| output.push_str(text), &|| {
                    cancel.load(Ordering::Acquire)
                });
            let elapsed = started.elapsed();
            let closed = server.join().unwrap();
            assert!(result.is_err());
            assert!(output.is_empty());
            assert!(
                elapsed < Duration::from_secs(1),
                "{provider} headers={headers}: cancellation took {elapsed:?}"
            );
            assert!(closed, "{provider} left inference connection open");
        }
    }
}
