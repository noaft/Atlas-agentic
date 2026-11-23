use atlas_agentic::{
    agents::LocalAgentDefinition,
    models::{provider_from_definition_with_settings, ModelProvider, ModelRequest},
    settings::AtlasSettings,
};
use serde_json::{json, Value};
use std::{
    io::{BufRead, BufReader, Read, Write},
    net::TcpListener,
    sync::mpsc,
    thread,
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
        temperature: Some(0.3),
        top_p: Some(0.9),
    }
}
type FakeProvider = (
    Box<dyn ModelProvider>,
    mpsc::Receiver<(String, Value)>,
    thread::JoinHandle<()>,
);

fn fake(provider: &str, response: &str, stream: bool) -> FakeProvider {
    fake_with_profile(provider, response, stream, json!({}))
}
fn fake_with_profile(provider: &str, response: &str, stream: bool, patch: Value) -> FakeProvider {
    fake_sequence(provider, &[(response, stream)], patch)
}
fn fake_sequence(provider: &str, responses: &[(&str, bool)], patch: Value) -> FakeProvider {
    fake_sequence_with_status(provider, responses, patch, 200)
}
fn fake_sequence_with_status(
    provider: &str,
    responses: &[(&str, bool)],
    patch: Value,
    status: u16,
) -> FakeProvider {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let responses: Vec<(String, bool)> = responses
        .iter()
        .map(|(body, stream)| (body.to_string(), *stream))
        .collect();
    let (tx, rx) = mpsc::channel();
    let server = thread::spawn(move || {
        for (response, stream) in responses {
            let (mut socket, _) = listener.accept().unwrap();
            let mut reader = BufReader::new(&mut socket);
            let mut headers = String::new();
            let mut length = 0;
            loop {
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                if line == "\r\n" {
                    break;
                }
                if let Some(v) = line.to_lowercase().strip_prefix("content-length:") {
                    length = v.trim().parse().unwrap();
                }
                headers.push_str(&line);
            }
            let mut body = vec![0; length];
            reader.read_exact(&mut body).unwrap();
            tx.send((
                headers,
                if body.is_empty() {
                    Value::Null
                } else {
                    serde_json::from_slice(&body).unwrap()
                },
            ))
            .unwrap();
            write!(socket,"HTTP/1.1 {status} Test\r\nContent-Type: {}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", if stream {"text/event-stream"} else {"application/json"}, response.len(), response).unwrap();
        }
    });
    let mut settings = serde_json::to_value(AtlasSettings::default()).unwrap();
    settings["selectedProvider"] = json!(provider);
    let mut profile = json!({"apiKey":"test-secret", "baseUrl":url,"model":"alias"});
    for (key, value) in patch.as_object().unwrap() {
        profile[key] = value.clone();
    }
    if let Some(limits) = profile
        .get_mut("modelLimits")
        .and_then(Value::as_object_mut)
    {
        let source = atlas_agentic::settings::providers::source_for(
            atlas_agentic::settings::ProviderId::parse(provider).unwrap(),
            &url,
        )
        .unwrap();
        for entry in limits.values_mut() {
            *entry = json!({"sourceId":source.source_id,"limits":entry.clone()});
        }
    }
    settings["providers"] = json!({provider:profile});
    let settings: AtlasSettings = serde_json::from_value(settings).unwrap();
    let effective = settings.resolve_with(|_| None).unwrap();
    let definition = LocalAgentDefinition::from_toml(&format!("name='test'\ndisplay_name='Test'\nmodel_provider='{provider}'\nmodel='alias'\nsystem_prompt='System'\nconversation_title_prompt='Title'\n")).unwrap();
    let model = provider_from_definition_with_settings(&definition, &effective).unwrap();
    (model, rx, server)
}
#[test]
fn native_and_compatible_payloads_use_their_protocol_and_no_deepseek_fields() {
    for (provider, response, path) in [
        (
            "openai",
            r#"{"status":"completed","output":[{"type":"message","content":[{"type":"output_text","text":"answer"}]}]}"#,
            "/responses",
        ),
        (
            "anthropic",
            r#"{"content":[{"type":"text","text":"answer"}],"stop_reason":"end_turn"}"#,
            "/messages",
        ),
        (
            "gemini",
            r#"{"candidates":[{"content":{"parts":[{"text":"answer"}]},"finishReason":"STOP"}]}"#,
            "/models/alias:generateContent",
        ),
        (
            "openai-compatible",
            r#"{"choices":[{"message":{"content":"answer"},"finish_reason":"stop"}]}"#,
            "/chat/completions",
        ),
    ] {
        let (model, rx, server) = fake(provider, response, false);
        assert_eq!(model.complete(&request()).unwrap(), "answer");
        let (headers, body) = rx.recv().unwrap();
        assert!(headers.starts_with(&format!("POST {path} ")), "{headers}");
        assert!(body.get("thinking").is_none());
        assert!(body.get("temperature").is_none());
        assert!(body.get("top_p").is_none());
        if provider == "openai" {
            assert_eq!(body["store"], false);
            assert_eq!(body["max_output_tokens"], 500);
        }
        if provider == "anthropic" {
            assert!(headers.to_lowercase().contains("x-api-key: test-secret"));
            assert_eq!(body["max_tokens"], 500);
        }
        if provider == "gemini" {
            assert!(headers
                .to_lowercase()
                .contains("x-goog-api-key: test-secret"));
            assert_eq!(body["generationConfig"]["maxOutputTokens"], 500);
        }
        server.join().unwrap();
    }
}
#[test]
fn native_stream_usage_includes_thinking_and_cache_once_and_preserves_alias() {
    for (provider, body, want) in [
        ("openai", concat!("event: response.output_text.delta\ndata: {\"type\":\"response.output_text.delta\",\"delta\":\"answer\"}\n\n", "data: {\"type\":\"response.completed\",\"response\":{\"model\":\"dated-model\",\"status\":\"completed\",\"usage\":{\"input_tokens\":1200,\"output_tokens\":300,\"total_tokens\":1500}}}\n\n"), (1200,300,1500)),
        ("anthropic", concat!("data: {\"type\":\"message_start\",\"message\":{\"usage\":{\"input_tokens\":100,\"cache_read_input_tokens\":200,\"cache_creation_input_tokens\":50,\"output_tokens\":0}}}\n\n", "data: {\"type\":\"content_block_delta\",\"delta\":{\"type\":\"text_delta\",\"text\":\"answer\"}}\n\n", "data: {\"type\":\"message_delta\",\"delta\":{},\"usage\":{\"output_tokens\":10}}\n\n", "data: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"end_turn\"},\"usage\":{\"output_tokens\":30}}\n\n", "data: {\"type\":\"message_stop\"}\n\n"), (350,30,380)),
        ("gemini", "data: {\"candidates\":[{\"content\":{\"parts\":[{\"text\":\"answer\"}]},\"finishReason\":\"STOP\"}],\"usageMetadata\":{\"promptTokenCount\":100,\"cachedContentTokenCount\":20,\"toolUsePromptTokenCount\":5,\"candidatesTokenCount\":30,\"thoughtsTokenCount\":10,\"totalTokenCount\":145}}\n\n", (105,40,145)),
        ("openai-compatible", "data: {\"choices\":[{\"delta\":{\"content\":\"answer\"},\"finish_reason\":\"stop\"}],\"usage\":{\"prompt_tokens\":1200,\"completion_tokens\":300,\"total_tokens\":1500}}\n\ndata: [DONE]\n\n", (1200,300,1500)),
    ] {
        let (model, rx, server) = fake(provider,body,true);
        let mut text = String::new();
        let result = model.stream_with_usage(&request(), &mut |s| text.push_str(s)).unwrap();
        assert_eq!(text,"answer"); assert_eq!(result.content,"answer");
        let usage = result.usage.unwrap(); assert_eq!(usage.model,"alias");
        assert_eq!((usage.input_tokens,usage.output_tokens,usage.total_tokens),want);
        rx.recv().unwrap(); server.join().unwrap();
    }
}
#[test]
fn stream_error_and_eof_do_not_succeed_with_partial_text() {
    for body in [
        "data: {\"type\":\"response.output_text.delta\",\"delta\":\"partial\"}\n\n",
        "data: {\"type\":\"error\",\"error\":{\"message\":\"test-secret\"}}\n\n",
    ] {
        let (model, rx, server) = fake("openai", body, true);
        let error = model
            .stream_with_usage(&request(), &mut |_| {})
            .err()
            .unwrap();
        assert!(!error.contains("test-secret"));
        rx.recv().unwrap();
        server.join().unwrap();
    }
}

#[test]
fn native_summary_truncation_is_explicit_and_uses_the_requested_output_bound() {
    use atlas_agentic::models::SummaryRequest;
    for (provider, response) in [
        (
            "openai",
            r#"{"status":"incomplete","incomplete_details":{"reason":"max_output_tokens"},"output":[{"type":"message","content":[{"type":"output_text","text":"partial"}]}]}"#,
        ),
        (
            "anthropic",
            r#"{"content":[{"type":"text","text":"partial"}],"stop_reason":"max_tokens"}"#,
        ),
        (
            "gemini",
            r#"{"candidates":[{"content":{"parts":[{"text":"partial"}]},"finishReason":"MAX_TOKENS"}]}"#,
        ),
        (
            "openai-compatible",
            r#"{"choices":[{"message":{"content":"partial"},"finish_reason":"length"}]}"#,
        ),
    ] {
        let (model, rx, server) = fake(provider, response, false);
        let result = model
            .summarize(&SummaryRequest {
                system_prompt: "Summarize",
                content: "History",
                max_output_tokens: 123,
            })
            .unwrap();
        assert_eq!(result.finish_reason, "length");
        let (_, payload) = rx.recv().unwrap();
        assert_eq!(
            payload[match provider {
                "openai" => "max_output_tokens",
                "gemini" => "generationConfig",
                _ => "max_tokens",
            }],
            if provider == "gemini" {
                json!({"maxOutputTokens":123})
            } else {
                json!(123)
            }
        );
        assert!(payload.get("temperature").is_none());
        server.join().unwrap();
    }
}

#[test]
fn invalid_provider_usage_is_unknown_instead_of_zero_or_an_estimate() {
    for usage in [
        json!(null),
        json!({"input_tokens":-1,"output_tokens":10,"total_tokens":9}),
        json!({"input_tokens":100,"output_tokens":10,"total_tokens":111}),
        json!({"input_tokens":u64::MAX,"output_tokens":1}),
    ] {
        let response=format!("data: {{\"type\":\"response.output_text.delta\",\"delta\":\"answer\"}}\n\ndata: {{\"type\":\"response.completed\",\"response\":{{\"status\":\"completed\",\"usage\":{usage}}}}}\n\n");
        let (model, rx, server) = fake("openai", &response, true);
        assert!(model
            .stream_with_usage(&request(), &mut |_| {})
            .unwrap()
            .usage
            .is_none());
        rx.recv().unwrap();
        server.join().unwrap();
    }
}

#[test]
fn compatible_keyless_endpoints_omit_authorization_and_optional_flags_are_explicit() {
    let (model, rx, server) = fake_with_profile(
        "openai-compatible",
        r#"{"choices":[{"message":{"content":"answer"},"finish_reason":"stop"}]}"#,
        false,
        json!({"apiKey":null,"allowKeyless":true,"capabilities":{"temperature":true,"topP":true}}),
    );
    assert_eq!(model.complete(&request()).unwrap(), "answer");
    let (headers, payload) = rx.recv().unwrap();
    assert!(!headers.to_lowercase().contains("authorization:"));
    assert!((payload["temperature"].as_f64().unwrap() - 0.3).abs() < 0.000001);
    assert!((payload["top_p"].as_f64().unwrap() - 0.9).abs() < 0.000001);
    assert!(payload.get("stream_options").is_none());
    assert!(payload.get("response_format").is_none());
    server.join().unwrap();
}

#[test]
fn images_use_native_payload_formats_and_provider_preflight_counts() {
    use atlas_agentic::media::{ImageData, ImageDetail};
    let images = [ImageData {
        mime_type: "image/png".into(),
        bytes: vec![1, 2, 3],
        width: 1,
        height: 1,
        detail: ImageDetail::Original,
    }];
    for (provider, response) in [
        (
            "openai",
            r#"{"status":"completed","output":[{"type":"message","content":[{"type":"output_text","text":"answer"}]}]}"#,
        ),
        (
            "anthropic",
            r#"{"content":[{"type":"text","text":"answer"}],"stop_reason":"end_turn"}"#,
        ),
        (
            "gemini",
            r#"{"candidates":[{"content":{"parts":[{"text":"answer"}]},"finishReason":"STOP"}]}"#,
        ),
        (
            "openai-compatible",
            r#"{"choices":[{"message":{"content":"answer"},"finish_reason":"stop"}]}"#,
        ),
    ] {
        let count = if provider == "gemini" {
            r#"{"totalTokens":42}"#
        } else {
            r#"{"input_tokens":42}"#
        };
        let responses = if provider == "openai-compatible" {
            vec![(response, false)]
        } else {
            vec![(count, false), (response, false)]
        };
        let (model, rx, server) = fake_sequence(
            provider,
            &responses,
            json!({"capabilities":{"imageInput":true}}),
        );
        let mut input = request();
        input.images = &images;
        let count = model.estimate_request(&input).unwrap();
        if provider == "openai-compatible" {
            assert!(count.is_none());
        } else {
            assert_eq!(count.unwrap().tokens, 42);
            let (_, counted) = rx.recv().unwrap();
            assert!(counted.to_string().contains("AQID"));
        }
        assert_eq!(model.complete(&input).unwrap(), "answer");
        let (_, body) = rx.recv().unwrap();
        match provider {
            "openai" => assert_eq!(
                body["input"][0]["content"][1]["image_url"],
                "data:image/png;base64,AQID"
            ),
            "anthropic" => assert_eq!(body["messages"][0]["content"][1]["source"]["data"], "AQID"),
            "gemini" => assert_eq!(
                body["contents"][0]["parts"][1]["inlineData"]["data"],
                "AQID"
            ),
            _ => assert_eq!(
                body["messages"][1]["content"][1]["image_url"]["url"],
                "data:image/png;base64,AQID"
            ),
        }
        server.join().unwrap();
    }
}

#[test]
fn successful_openai_response_with_null_error_is_usable_for_titles_and_summaries() {
    let response = r#"{"status":"completed","error":null,"incomplete_details":null,"output":[{"type":"message","content":[{"type":"output_text","text":"answer"}]}]}"#;
    let (model, rx, server) = fake("openai", response, false);
    assert_eq!(model.complete(&request()).unwrap(), "answer");
    rx.recv().unwrap();
    server.join().unwrap();
    let (model, rx, server) = fake("openai", response, false);
    let summary = model
        .summarize(&atlas_agentic::models::SummaryRequest {
            system_prompt: "Summarize",
            content: "History",
            max_output_tokens: 100,
        })
        .unwrap();
    assert_eq!(summary.finish_reason, "stop");
    assert_eq!(summary.content, "answer");
    rx.recv().unwrap();
    server.join().unwrap();
}

#[test]
fn discovered_output_cap_clamps_a_title_even_without_combined_context_metadata() {
    let catalog = r#"{"data":[{"id":"alias","max_output_tokens":64}]}"#;
    let completion = r#"{"status":"completed","output":[{"type":"message","content":[{"type":"output_text","text":"answer"}]}]}"#;
    let (model, rx, server) = fake_sequence(
        "openai",
        &[(catalog, false), (completion, false)],
        json!({}),
    );
    let models = model.list_models().unwrap();
    assert_eq!(models[0].context_window, None);
    let mut title = request();
    title.max_output_tokens = None;
    assert_eq!(model.complete(&title).unwrap(), "answer");
    let _ = rx.recv().unwrap();
    let (_, payload) = rx.recv().unwrap();
    assert_eq!(payload["max_output_tokens"], 64);
    server.join().unwrap();
}

#[test]
fn known_sampling_rejection_overrides_explicit_profile_opt_in() {
    let catalog = r#"{"data":[{"id":"alias","capabilities":{"temperature":{"supported":false},"top_p":{"supported":false}}}]}"#;
    let completion = r#"{"choices":[{"message":{"content":"answer"},"finish_reason":"stop"}]}"#;
    let (model, rx, server) = fake_sequence(
        "openai-compatible",
        &[(catalog, false), (completion, false)],
        json!({"capabilities":{"temperature":true,"topP":true}}),
    );
    model.list_models().unwrap();
    model.complete(&request()).unwrap();
    let _ = rx.recv().unwrap();
    let (_, payload) = rx.recv().unwrap();
    assert!(payload.get("temperature").is_none());
    assert!(payload.get("top_p").is_none());
    server.join().unwrap();
}

#[test]
fn known_image_rejection_overrides_explicit_profile_opt_in() {
    use atlas_agentic::media::{ImageData, ImageDetail};
    let catalog = r#"{"data":[{"id":"alias","capabilities":{"image_input":{"supported":false}}}]}"#;
    let (model, rx, server) = fake_sequence(
        "openai-compatible",
        &[(catalog, false)],
        json!({"capabilities":{"imageInput":true}}),
    );
    model.list_models().unwrap();
    let image = ImageData {
        mime_type: "image/png".into(),
        bytes: vec![1, 2, 3],
        width: 1,
        height: 1,
        detail: ImageDetail::Original,
    };
    assert!(model.validate_images(&[image]).is_err());
    rx.recv().unwrap();
    server.join().unwrap();
}

#[test]
fn malformed_sse_json_shape_returns_an_error_without_panicking_the_worker() {
    let (model, rx, server) = fake("openai", "event: message\ndata: 42\n\n", true);
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        model.stream_with_usage(&request(), &mut |_| {})
    }));
    assert!(result.is_ok(), "malformed provider data must not panic");
    assert!(result.unwrap().is_err());
    rx.recv().unwrap();
    server.join().unwrap();
}

#[test]
fn gemini_missing_output_usage_is_unknown_instead_of_zero() {
    let body="data: {\"candidates\":[{\"content\":{\"parts\":[{\"text\":\"answer\"}]},\"finishReason\":\"STOP\"}],\"usageMetadata\":{\"promptTokenCount\":100}}\n\n";
    let (model, rx, server) = fake("gemini", body, true);
    assert!(model
        .stream_with_usage(&request(), &mut |_| {})
        .unwrap()
        .usage
        .is_none());
    rx.recv().unwrap();
    server.join().unwrap();
}

#[test]
fn native_preflight_counts_the_same_context_as_inference_and_caches_only_identical_input() {
    use atlas_agentic::{
        conversations::{ConversationMessage, MemoryBlock, MessageRole},
        tools::ToolDefinition,
    };
    for (provider, count_response, answer, path) in [
        (
            "openai",
            r#"{"input_tokens":321}"#,
            r#"{"status":"completed","output":[{"type":"message","content":[{"type":"output_text","text":"answer"}]}]}"#,
            "/responses/input_tokens",
        ),
        (
            "anthropic",
            r#"{"input_tokens":321}"#,
            r#"{"content":[{"type":"text","text":"answer"}],"stop_reason":"end_turn"}"#,
            "/messages/count_tokens",
        ),
        (
            "gemini",
            r#"{"totalTokens":321}"#,
            r#"{"candidates":[{"content":{"parts":[{"text":"answer"}]},"finishReason":"STOP"}]}"#,
            "/models/alias:countTokens",
        ),
    ] {
        let (model, rx, server) = fake_sequence(
            provider,
            &[
                (count_response, false),
                (answer, false),
                (count_response, false),
            ],
            json!({}),
        );
        let history = [ConversationMessage {
            role: MessageRole::User,
            content: "prior question".into(),
            created_at: chrono::Utc::now(),
        }];
        let memory = [MemoryBlock {
            start_index: 0,
            end_index_exclusive: 2,
            content: "verified memory".into(),
        }];
        let tools = [ToolDefinition {
            name: "sample_tool".into(),
            description: "tool description".into(),
            input_schema: json!({"type":"object"}),
        }];
        let mut input = request();
        input.history = &history;
        input.memory_blocks = &memory;
        input.tools = &tools;
        input.activity_context = Some("prior tool evidence");
        input.user_prompt = r#"{"type":"tool_results","result":"current tool evidence"}"#;
        let count = model.estimate_request(&input).unwrap().unwrap();
        assert_eq!(count.tokens, 321);
        assert_eq!(count.estimated, provider == "anthropic");
        assert_eq!(model.estimate_request(&input).unwrap().unwrap().tokens, 321);
        model.complete(&input).unwrap();
        let (headers, counted) = rx.recv().unwrap();
        let (_, generated) = rx.recv().unwrap();
        assert!(headers.starts_with(&format!("POST {path} ")), "{headers}");
        match provider {
            "openai" => {
                assert!(headers
                    .to_lowercase()
                    .contains("authorization: bearer test-secret"));
                assert_eq!(
                    counted,
                    json!({"model":generated["model"],"instructions":generated["instructions"],"input":generated["input"]})
                );
            }
            "anthropic" => {
                assert!(headers.to_lowercase().contains("x-api-key: test-secret"));
                assert!(headers.to_lowercase().contains("anthropic-version:"));
                assert_eq!(
                    counted,
                    json!({"model":generated["model"],"system":generated["system"],"messages":generated["messages"]})
                );
            }
            _ => {
                assert!(headers
                    .to_lowercase()
                    .contains("x-goog-api-key: test-secret"));
                assert_eq!(
                    counted["generateContentRequest"]["contents"],
                    generated["contents"]
                );
                assert_eq!(
                    counted["generateContentRequest"]["systemInstruction"],
                    generated["systemInstruction"]
                );
                assert_eq!(counted["generateContentRequest"]["model"], "models/alias");
            }
        }
        let rendered = counted.to_string();
        for text in [
            "prior question",
            "verified memory",
            "sample_tool",
            "prior tool evidence",
            "current tool evidence",
        ] {
            assert!(rendered.contains(text), "{rendered}");
        }
        input.user_prompt = "new event";
        model.estimate_request(&input).unwrap().unwrap();
        let (_, changed) = rx.recv().unwrap();
        assert_ne!(counted, changed);
        server.join().unwrap();
    }
}

#[test]
fn native_preflight_rejects_invalid_counts_without_echoing_provider_data() {
    for provider in ["openai", "anthropic", "gemini"] {
        for body in [
            r#"{"error":{"message":"test-secret"}}"#,
            r#"{"input_tokens":-1,"totalTokens":-1}"#,
            r#"{"input_tokens":"test-secret","totalTokens":"test-secret"}"#,
            r#"[]"#,
        ] {
            let (model, rx, server) = fake(provider, body, false);
            let error = model
                .estimate_request(&request())
                .expect_err("invalid count must fail");
            assert!(!error.contains("test-secret"));
            rx.recv().unwrap();
            server.join().unwrap();
        }
    }
}

#[test]
fn native_preflight_can_be_cancelled_before_headers_or_during_silent_body() {
    use std::{
        sync::{
            atomic::{AtomicBool, Ordering},
            Arc,
        },
        time::{Duration, Instant},
    };
    for headers in [false, true] {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let cancelled = Arc::new(AtomicBool::new(false));
        let flag = cancelled.clone();
        let server = thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            let mut bytes = [0; 4096];
            assert!(socket.read(&mut bytes).unwrap() > 0);
            if headers {
                socket
                    .write_all(
                        b"HTTP/1.1 200 OK\r\nContent-Length: 100\r\nConnection: close\r\n\r\n",
                    )
                    .unwrap();
                socket.flush().unwrap();
            }
            flag.store(true, Ordering::SeqCst);
            socket
                .set_read_timeout(Some(Duration::from_secs(3)))
                .unwrap();
            // Transport cancellation must drop the connection without a detached worker.
            loop {
                match socket.read(&mut bytes) {
                    Ok(0) => break,
                    Ok(_) => {}
                    Err(error) => panic!("count transport stayed open: {error}"),
                }
            }
        });
        let mut settings = serde_json::to_value(AtlasSettings::default()).unwrap();
        settings["selectedProvider"] = json!("openai");
        settings["providers"] =
            json!({"openai":{"apiKey":"test-secret","baseUrl":url,"model":"alias"}});
        let settings: AtlasSettings = serde_json::from_value(settings).unwrap();
        let definition = LocalAgentDefinition::from_toml("name='test'\ndisplay_name='Test'\nmodel_provider='openai'\nmodel='alias'\nsystem_prompt='System'\nconversation_title_prompt='Title'").unwrap();
        let model = provider_from_definition_with_settings(
            &definition,
            &settings.resolve_with(|_| None).unwrap(),
        )
        .unwrap();
        let started = Instant::now();
        assert!(model
            .estimate_request_with_control(&request(), &|| cancelled.load(Ordering::SeqCst))
            .unwrap_err()
            .contains("interrupted"));
        assert!(started.elapsed() < Duration::from_secs(2));
        server.join().unwrap();
    }
}

#[test]
fn gemini_preflight_uses_output_reserve_without_a_second_count_request() {
    use atlas_agentic::{
        agents::{AgentRegistry, LocalAgent},
        conversations::{ConversationService, ConversationStore},
        Runtime,
    };
    let stream="data: {\"candidates\":[{\"content\":{\"parts\":[{\"text\":\"done\"}]},\"finishReason\":\"STOP\"}]}\n\n";
    let (model, rx, server) = fake_sequence(
        "gemini",
        &[
            (r#"{"models":[]}"#, false),
            (r#"{"totalTokens":10}"#, false),
            (stream, true),
        ],
        json!({"modelLimits":{"alias":{"contextWindow":100000,"maxOutputTokens":4096}}}),
    );
    model.list_models().unwrap();
    rx.recv().unwrap();
    let definition=LocalAgentDefinition::from_toml("name='test'\ndisplay_name='Test'\nmodel_provider='gemini'\nmodel='alias'\nsystem_prompt='assistant'\nconversation_title_prompt='title'\nconversation_summary_prompt='summary'\n[memory]\nenabled=true\noutput_reserve_tokens=123").unwrap();
    let runtime = Runtime::new(AgentRegistry::new(vec![Box::new(LocalAgent::new(
        definition, model,
    ))]));
    let data = tempfile::tempdir().unwrap();
    let store = ConversationStore::at(data.path().into());
    let meta = store.create_turn("old", "answer").unwrap();
    ConversationService::new(&runtime, &store, 40)
        .ask("hello", Some(&meta.id), &mut |_| {})
        .unwrap();
    let (headers, counted) = rx.recv().unwrap();
    assert!(headers.contains(":countTokens "));
    let (headers, sent) = rx.recv().unwrap();
    assert!(headers.contains(":streamGenerateContent?alt=sse "));
    assert_eq!(
        counted["generateContentRequest"]["generationConfig"]["maxOutputTokens"],
        123
    );
    assert_eq!(sent["generationConfig"]["maxOutputTokens"], 123);
    server.join().unwrap();
}

#[test]
fn native_preflight_http_errors_are_sanitized_and_never_fall_back() {
    for status in [404, 429, 500] {
        let (model, rx, server) = fake_sequence_with_status(
            "openai",
            &[(r#"{"error":{"message":"test-secret"}}"#, false)],
            json!({}),
            status,
        );
        let error = model.estimate_request(&request()).unwrap_err();
        assert!(error.contains(&format!("HTTP {status}")), "{error}");
        assert!(!error.contains("test-secret"));
        rx.recv().unwrap();
        server.join().unwrap();
    }
}
#[test]
fn native_preflight_bounds_response_bytes_and_does_not_cache_failures() {
    let oversized = " ".repeat(65537);
    for invalid in ["not JSON", oversized.as_str()] {
        let (model, rx, server) = fake_sequence(
            "openai",
            &[(invalid, false), (r#"{"input_tokens":42}"#, false)],
            json!({}),
        );
        assert!(model.estimate_request(&request()).is_err());
        assert_eq!(
            model.estimate_request(&request()).unwrap().unwrap().tokens,
            42
        );
        rx.recv().unwrap();
        rx.recv().unwrap();
        server.join().unwrap();
    }
}
