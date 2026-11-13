use atlas_agentic::agent_output::AgentOutput;

#[test]
fn hard_and_lazy_wakeup_policies_are_parsed_and_bounded() {
    for (policy, milliseconds) in [
        (r#"{"mode":"hard","interval_seconds":0.1}"#, 100),
        (r#"{"mode":"hard","interval_seconds":29.999}"#, 29999),
        (r#"{"mode":"lazy"}"#, 600000),
    ] {
        let input =
            format!(r#"{{"type":"tool_call","tool":"pwd","wake_up":{policy},"args":{{}}}}"#);
        let parsed = AgentOutput::parse_tool_protocol(&input).unwrap();
        let AgentOutput::ScheduledToolCall { execution, .. } = parsed else {
            panic!("wake_up must produce a managed call");
        };
        assert_eq!(execution.interval_ms().unwrap(), milliseconds);
    }
}

#[test]
fn invalid_wakeup_policies_are_rejected() {
    for policy in [
        r#"{"mode":"hard"}"#,
        r#"{"mode":"hard","interval_seconds":0.099}"#,
        r#"{"mode":"hard","interval_seconds":30}"#,
        r#"{"mode":"hard","interval_seconds":-2}"#,
        r#"{"mode":"hard","interval_seconds":1e999}"#,
        r#"{"mode":"lazy","interval_seconds":2}"#,
        r#"{"mode":"lazy","interval_seconds":null}"#,
        r#"{"mode":"job","yield_ms":2000}"#,
        r#"{"mode":"hard","interval_seconds":2,"extra":true}"#,
    ] {
        let input =
            format!(r#"{{"type":"tool_call","tool":"pwd","wake_up":{policy},"args":{{}}}}"#);
        assert!(
            AgentOutput::parse_tool_protocol(&input).is_err(),
            "accepted {policy}"
        );
    }
}

#[test]
fn legacy_execution_policies_require_explicit_migration() {
    let input = r#"{"type":"tool_call","tool":"pwd","execution":{"mode":"task","yield_ms":2000},"args":{}}"#;
    let error = AgentOutput::parse_tool_protocol(input).unwrap_err();
    assert!(
        error.contains("execution") && error.contains("wake_up"),
        "{error}"
    );
    assert!(AgentOutput::parse(input).is_err());
}

#[test]
fn wakeup_updates_validate_the_target_and_policy() {
    let input = r#"{"type":"set_execution_wakeup","execution_id":"owned","wake_up":{"mode":"hard","interval_seconds":0.2}}"#;
    assert!(matches!(
        AgentOutput::parse_tool_protocol(input).unwrap(),
        AgentOutput::Execution { .. }
    ));
    for input in [
        r#"{"type":"set_execution_wakeup","execution_id":"","wake_up":{"mode":"lazy"}}"#,
        r#"{"type":"set_execution_wakeup","execution_id":"owned","wake_up":{"mode":"hard"}}"#,
    ] {
        assert!(AgentOutput::parse_tool_protocol(input).is_err());
    }
}

#[test]
fn default_wakeup_and_prompt_share_configuration() {
    use atlas_agentic::executions::protocol::{wakeup_prompt, ExecutionPolicy, WakeupMode};
    let default = ExecutionPolicy::default();
    assert_eq!(default.mode, WakeupMode::Hard);
    assert_eq!(default.interval_ms().unwrap(), 2000);
    let prompt = wakeup_prompt();
    assert!(prompt.contains(&serde_json::to_string(&default).unwrap()));
    assert!(prompt.contains("every 600 seconds"));
    assert!(!prompt.contains("{{"));
    for seconds in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        assert!(ExecutionPolicy {
            mode: WakeupMode::Hard,
            interval_seconds: Some(seconds)
        }
        .validate()
        .is_err());
    }
}
