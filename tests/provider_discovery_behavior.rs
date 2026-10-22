use atlas_agentic::{
    models::discovery::discover_models,
    settings::{AtlasSettings, ProviderId, ProviderSettings},
};
use serde_json::json;
use std::{
    io::{BufRead, BufReader, Write},
    net::TcpListener,
    thread,
};
#[test]
fn discovery_needs_no_selected_model_and_never_invents_combined_capacity() {
    for (provider, response) in [
        (
            ProviderId::Gemini,
            json!({"models":[{"name":"models/chat","displayName":"Chat","inputTokenLimit":100000,"outputTokenLimit":8000,"supportedGenerationMethods":["generateContent"]},{"name":"models/embed","supportedGenerationMethods":["embedContent"]}]}),
        ),
        (
            ProviderId::Anthropic,
            json!({"data":[{"id":"chat","display_name":"Chat","max_input_tokens":100000,"max_tokens":8000}],"has_more":false}),
        ),
        (
            ProviderId::Openai,
            json!({"data":[{"id":"chat"},{"id":"chat"},{"id":""}]}),
        ),
    ] {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let server = thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            let mut reader = BufReader::new(&mut socket);
            let mut line = String::new();
            reader.read_line(&mut line).unwrap();
            assert!(line.starts_with("GET /models "));
            loop {
                line.clear();
                reader.read_line(&mut line).unwrap();
                if line == "\r\n" {
                    break;
                }
            }
            let body = response.to_string();
            write!(socket,"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",body.len(),body).unwrap();
        });
        let mut settings = AtlasSettings {
            selected_provider: Some(provider),
            ..Default::default()
        };
        settings.providers.insert(
            provider.as_str().into(),
            ProviderSettings {
                api_key: Some("fake".into()),
                base_url: url,
                ..Default::default()
            },
        );
        let effective = settings.resolve_with(|_| None).unwrap();
        assert!(effective.model.is_empty());
        let models = discover_models(&effective).unwrap();
        assert_eq!(models.len(), 1);
        assert_eq!(models[0].context_window, None);
        if provider != ProviderId::Openai {
            assert_eq!(models[0].input_token_limit, Some(100000));
            assert_eq!(models[0].max_output_tokens, Some(8000));
        }
        assert_eq!(models[0].source.as_ref().unwrap().provider, provider);
        server.join().unwrap();
    }
}
#[test]
fn source_normalization_is_stable_but_separates_endpoints() {
    use atlas_agentic::settings::providers::source_for;
    assert_eq!(
        source_for(ProviderId::Openai, "https://EXAMPLE.test/v1/").unwrap(),
        source_for(ProviderId::Openai, "https://example.test/v1").unwrap()
    );
    assert_ne!(
        source_for(ProviderId::Openai, "https://example.test/v1").unwrap(),
        source_for(ProviderId::Openai, "https://example.test/v2").unwrap()
    );
}

#[test]
fn discovery_follows_pagination_and_rejects_repeated_cursors_without_looping() {
    use std::sync::mpsc;
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let (tx, rx) = mpsc::channel();
    let server = thread::spawn(move || {
        for _ in 0..2 {
            let (mut socket, _) = listener.accept().unwrap();
            let mut reader = BufReader::new(&mut socket);
            let mut first = String::new();
            reader.read_line(&mut first).unwrap();
            tx.send(first).unwrap();
            loop {
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                if line == "\r\n" {
                    break;
                }
            }
            let body = r#"{"data":[{"id":"alias"}],"has_more":true,"last_id":"repeated"}"#;
            write!(socket,"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",body.len(),body).unwrap();
        }
    });
    let mut settings = AtlasSettings {
        selected_provider: Some(ProviderId::Anthropic),
        ..Default::default()
    };
    settings.providers.insert(
        "anthropic".into(),
        ProviderSettings {
            api_key: Some("fake".into()),
            base_url: url,
            ..Default::default()
        },
    );
    let error = discover_models(&settings.resolve_with(|_| None).unwrap()).unwrap_err();
    assert!(error.contains("repeated a pagination cursor"));
    assert!(rx.recv().unwrap().starts_with("GET /models "));
    assert!(rx
        .recv()
        .unwrap()
        .starts_with("GET /models?after_id=repeated "));
    server.join().unwrap();
}
