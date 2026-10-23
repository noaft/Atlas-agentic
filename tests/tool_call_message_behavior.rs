use atlas_agentic::agent_output::AgentOutput;
use serde_json::json;

#[test]
fn optional_tool_message_rejects_invalid_or_oversized_text() {
    for message in [
        json!(""),
        json!("  "),
        json!(12),
        json!({}),
        json!("🦀".repeat(1001)),
    ] {
        let response = json!({"type":"tool_call", "tool":"pwd", "args":{}, "message":message});
        // Serialize with type first, as required by the runtime protocol.
        let response = format!(
            r#"{{"type":"tool_call","tool":"pwd","args":{{}},"message":{}}}"#,
            response["message"]
        );
        assert!(
            AgentOutput::parse_tool_protocol(&response).is_err(),
            "{response}"
        );
    }
}

#[test]
fn optional_tool_message_preserves_legacy_calls_and_trims_valid_unicode() {
    for (field, expected) in [
        (String::new(), None),
        (",\"message\":null".into(), None),
        (
            ",\"message\":\"  Kiểm tra mã nguồn 🚀  \"".into(),
            Some("Kiểm tra mã nguồn 🚀".to_string()),
        ),
        (
            format!(",\"message\":{}", json!("🦀".repeat(1000))),
            Some("🦀".repeat(1000)),
        ),
    ] {
        let response = format!(r#"{{"type":"tool_call","tool":"pwd","args":{{}}{field}}}"#);
        match AgentOutput::parse_tool_protocol(&response).unwrap() {
            AgentOutput::ToolCall { message, .. } => assert_eq!(message, expected),
            _ => panic!("expected tool call"),
        }
    }
}
