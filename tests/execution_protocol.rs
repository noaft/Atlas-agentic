use atlas_agentic::agent_output::AgentOutput;

#[test]
fn execution_controls_accept_the_normal_tool_call_envelope() {
    for (name, args) in [
        (
            "read_execution",
            r#"{"execution_id":"owned","tail_lines":20}"#,
        ),
        ("await_execution", r#"{"execution_ids":["owned"]}"#),
        ("list_executions", "{}"),
        ("cancel_execution", r#"{"execution_id":"owned"}"#),
        (
            "set_execution_wakeup",
            r#"{"execution_id":"owned","wake_up":{"mode":"lazy"}}"#,
        ),
    ] {
        let top_level = format!(r#"{{"type":"{name}",{} }}"#, &args[1..args.len() - 1]);
        let top_level = if args == "{}" {
            format!(r#"{{"type":"{name}"}}"#)
        } else {
            top_level
        };
        let wrapped = format!(r#"{{"type":"tool_call","tool":"{name}","args":{args}}}"#);
        assert_eq!(
            AgentOutput::parse_tool_protocol(&wrapped).unwrap(),
            AgentOutput::parse_tool_protocol(&top_level).unwrap(),
            "{name} must route to runtime control instead of the tool registry"
        );
    }
}

#[test]
fn wrapped_execution_controls_validate_arguments_and_cannot_start_a_worker() {
    for input in [
        r#"{"type":"tool_call","tool":"read_execution","args":{"execution_id":"owned","tail_lines":201}}"#,
        r#"{"type":"tool_call","tool":"read_execution","args":{"execution_id":"owned","tail_lines":1,"cursor":0}}"#,
        r#"{"type":"tool_call","tool":"read_execution","args":{"execution_id":"owned","owner":"other-chat"}}"#,
        r#"{"type":"tool_call","tool":"list_executions","args":{"type":"cancel_execution"}}"#,
        r#"{"type":"tool_call","tool":"read_execution","wake_up":{"mode":"lazy"},"args":{"execution_id":"owned"}}"#,
        r#"{"type":"tool_call","tool":"list_executions","args":[]}"#,
    ] {
        assert!(
            AgentOutput::parse_tool_protocol(input).is_err(),
            "accepted invalid control {input}"
        );
    }
}

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
