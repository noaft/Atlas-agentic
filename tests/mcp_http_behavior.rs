use atlas_agentic::{
    agent_output::AgentOutput,
    mcp::{register_tools, McpConfig},
    tools::ToolRegistry,
};
use serde_json::{json, Value};
use std::{
    io::{BufRead, BufReader, Read, Write},
    net::TcpListener,
    sync::{Arc, Mutex},
    thread,
};

#[test]
fn http_configuration_validates_transport_and_oauth() {
    for input in [
        "url='https://example.com/mcp'",
        "url='http://127.0.0.1:1234/mcp'",
        "url='http://[::1]:1234/mcp'",
        "url='https://example.com/mcp'\n[servers.demo.oauth]\nclient_id='atlas'\nscopes=['tools']",
    ] {
        assert!(
            McpConfig::from_toml(&format!("[servers.demo]\n{input}")).is_ok(),
            "{input}"
        );
    }
    for input in [
        "url='http://example.com'",
        "url='https://user:secret@example.com'",
        "url='https://example.com/#x'",
        "url='https://example.com/?token=secret'",
        "url='https://example.com'\ncommand='server'",
        "command='server'\n[servers.demo.oauth]",
        "url='https://example.com'\n[servers.demo.oauth]\nredirect_port=0",
        "url='https://example.com'\n[servers.demo.oauth]\nscopes=[' ']",
    ] {
        assert!(
            McpConfig::from_toml(&format!("[servers.demo]\n{input}")).is_err(),
            "{input}"
        );
    }
}

fn execute(registry: &ToolRegistry, tool: &str) -> Result<Value, String> {
    let args = if tool == "mcp_call_tool" {
        json!({"server":"demo","tool":"echo","arguments":{}})
    } else {
        json!({"server":"demo"})
    };
    let result = registry.execute(&AgentOutput::ToolCall {
        tool: tool.into(),
        args,
        message: None,
    })?;
    serde_json::from_str(&result.message).map_err(|_| "bad output".into())
}

type Records = Arc<Mutex<Vec<(String, Value)>>>;

// A real loopback server records requests and never requires external credentials.
fn fixture(scenario: &str) -> (ToolRegistry, Records, thread::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}/mcp", listener.local_addr().unwrap());
    let config =
        McpConfig::from_toml(&format!("[servers.demo]\nurl='{url}'\ntimeout_ms=500")).unwrap();
    let mut registry = ToolRegistry::empty();
    register_tools(&mut registry, config, std::path::Path::new("."));
    let records = Arc::new(Mutex::new(Vec::new()));
    let captured = records.clone();
    let scenario = scenario.to_string();
    let worker = thread::spawn(move || {
        let count = if matches!(scenario.as_str(), "json" | "sse" | "sse_notification") {
            4
        } else if scenario == "call_timeout" {
            3
        } else {
            1
        };
        for _ in 0..count {
            let (mut stream, _) = listener.accept().unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut headers = String::new();
            loop {
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                headers.push_str(&line);
                if line == "\r\n" {
                    break;
                }
            }
            let length: usize = headers
                .lines()
                .find_map(|line| {
                    line.to_ascii_lowercase()
                        .strip_prefix("content-length:")
                        .map(|n| n.trim().parse().unwrap())
                })
                .unwrap_or(0);
            let mut body = vec![0; length];
            reader.read_exact(&mut body).unwrap();
            let message: Value = serde_json::from_slice(&body).unwrap();
            captured.lock().unwrap().push((headers, message.clone()));
            let method = message["method"].as_str().unwrap();
            if scenario == "call_timeout" && method == "tools/call" {
                thread::sleep(std::time::Duration::from_millis(800));
                continue;
            }
            let result = match method {
                "initialize" => {
                    json!({"protocolVersion":"2025-11-25","capabilities":{"tools":{}},"serverInfo":{"name":"fixture","version":"1"}})
                }
                "tools/list" => json!({"tools":[{"name":"echo","inputSchema":{"type":"object"}}]}),
                _ => json!({"content":[{"type":"text","text":"ok"}]}),
            };
            let mut response = json!({"jsonrpc":"2.0","id":message["id"],"result":result});
            if scenario == "wrong_id" {
                response["id"] = json!(999);
            }
            if scenario == "protocol_error" {
                response = json!({"jsonrpc":"2.0","id":message["id"],"error":{"code":-1,"message":"secret"}});
            }
            let (status, content_type, body) = if method == "notifications/initialized" {
                ("202 Accepted", "application/json", String::new())
            } else {
                match scenario.as_str() {
                    "auth" => ("401 Unauthorized", "text/plain", "secret".into()),
                    "redirect" => ("307 Temporary Redirect", "text/plain", "secret".into()),
                    "oversized" => ("200 OK", "application/json", "x".repeat(1048577)),
                    "sse_oversized" => ("200 OK", "text/event-stream", format!("data: {}", "x".repeat(1048577))),
                    "server_request" => ("200 OK", "text/event-stream", "data: {\"jsonrpc\":\"2.0\",\"id\":42,\"method\":\"sampling/createMessage\"}\n\n".into()),
                    "sse_notification" => ("200 OK", "text/event-stream", format!("data: {{\"jsonrpc\":\"2.0\",\"method\":\"notifications/message\"}}\n\ndata: {response}\n\n")),
                    "sse" => (
                        "200 OK",
                        "text/event-stream",
                        format!("event: message\r\ndata: {response}\r\n\r\n"),
                    ),
                    _ => ("200 OK", "application/json", response.to_string()),
                }
            };
            let _=write!(stream,"HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nMcp-Session-Id: fixture-session\r\nLocation: /other\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len());
        }
    });
    (registry, records, worker)
}

#[test]
fn http_handshake_sessions_json_and_sse_tool_calls() {
    for scenario in ["json", "sse", "sse_notification"] {
        let (registry, records, worker) = fixture(scenario);
        assert_eq!(
            execute(&registry, "mcp_list_tools").unwrap()["tools"][0]["name"],
            "echo"
        );
        assert_eq!(
            execute(&registry, "mcp_call_tool").unwrap()["content"][0]["text"],
            "ok"
        );
        worker.join().unwrap();
        let records = records.lock().unwrap();
        assert_eq!(records[1].1["method"], "notifications/initialized");
        for (headers, _) in &records[1..] {
            let headers = headers.to_ascii_lowercase();
            assert!(headers.contains("mcp-session-id: fixture-session"));
            assert!(headers.contains("mcp-protocol-version: 2025-11-25"));
        }
    }
}

#[test]
fn http_errors_are_sanitized_and_requests_are_never_replayed() {
    for (scenario, expected) in [
        ("auth", "Connect"),
        ("redirect", "HTTP"),
        ("wrong_id", "identity"),
        ("protocol_error", "protocol error"),
        ("oversized", "1 MiB"),
        ("sse_oversized", "1 MiB"),
        ("server_request", "unsupported client method"),
    ] {
        let (registry, records, worker) = fixture(scenario);
        let error = execute(&registry, "mcp_list_tools").unwrap_err();
        assert!(error.contains(expected), "{scenario}: {error}");
        assert!(!error.contains("secret"));
        worker.join().unwrap();
        assert_eq!(records.lock().unwrap().len(), 1);
    }
}

#[test]
fn http_authorization_and_registration_never_connect() {
    use atlas_agentic::permissions::ToolRisk;
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let config = McpConfig::from_toml(&format!(
        "[servers.demo]\nurl='http://{}/mcp'",
        listener.local_addr().unwrap()
    ))
    .unwrap();
    let mut registry = ToolRegistry::empty();
    register_tools(&mut registry, config, std::path::Path::new("."));
    let auth = registry
        .authorization("mcp_list_tools", &json!({"server":"demo"}))
        .unwrap();
    assert_eq!(auth.risk(), ToolRisk::ExternalTool);
    assert!(listener.accept().is_err());
}

#[test]
fn http_call_timeout_reports_unknown_outcome_without_replay() {
    let (registry, records, worker) = fixture("call_timeout");
    let start = std::time::Instant::now();
    let error = execute(&registry, "mcp_call_tool").unwrap_err();
    assert!(error.contains("outcome may be unknown"), "{error}");
    assert!(start.elapsed() < std::time::Duration::from_secs(2));
    worker.join().unwrap();
    let records = records.lock().unwrap();
    assert_eq!(records.len(), 3);
    assert_eq!(records[2].1["method"], "tools/call");
}
