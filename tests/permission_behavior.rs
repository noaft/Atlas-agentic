use std::{
    cell::{Cell, RefCell},
    collections::VecDeque,
    rc::Rc,
};

use atlas_agentic::{
    agents::{AgentRegistry, LocalAgent, LocalAgentDefinition},
    models::{ModelProvider, ModelRequest},
    permissions::{
        requires_confirmation, AuthorizationDecision, AuthorizationRequest, PermissionMode,
        ToolAuthorizer, ToolEvent, ToolRisk,
    },
    tools::{summarize_arguments, Tool, ToolDefinition, ToolRegistry, ToolResult},
    Runtime,
};

struct QueueModel {
    responses: RefCell<VecDeque<&'static str>>,
    requests: Rc<RefCell<Vec<String>>>,
}

impl ModelProvider for QueueModel {
    fn complete(&self, request: &ModelRequest) -> Result<String, String> {
        self.requests
            .borrow_mut()
            .push(request.user_prompt.to_string());
        Ok(self.responses.borrow_mut().pop_front().unwrap().to_string())
    }
}

struct RiskyTool {
    executions: Rc<Cell<usize>>,
}

impl Tool for RiskyTool {
    fn name(&self) -> &str {
        "risky"
    }

    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: self.name().to_string(),
            description: "A modifying test tool.".to_string(),
            input_schema: serde_json::json!({"type":"object"}),
        }
    }

    fn risk(&self, _args: &serde_json::Value) -> Result<ToolRisk, String> {
        Ok(ToolRisk::ModifyWorkspace)
    }

    fn execute(&self, _args: &serde_json::Value) -> Result<ToolResult, String> {
        self.executions.set(self.executions.get() + 1);
        Ok(ToolResult::text("changed"))
    }
}

struct FixedAuthorizer {
    decision: Result<AuthorizationDecision, &'static str>,
    calls: Cell<usize>,
}

impl ToolAuthorizer for FixedAuthorizer {
    fn authorize(&self, _request: &AuthorizationRequest) -> Result<AuthorizationDecision, String> {
        self.calls.set(self.calls.get() + 1);
        self.decision.map_err(str::to_string)
    }
}

fn test_runtime(executions: Rc<Cell<usize>>) -> (Runtime, Rc<RefCell<Vec<String>>>) {
    let definition = LocalAgentDefinition::from_toml(
        r#"
name = "atlas"
display_name = "Atlas"
model_provider = "fake"
model = "fake"
system_prompt = "You are Atlas."
conversation_title_prompt = "Write a title."
"#,
    )
    .unwrap();
    let requests = Rc::new(RefCell::new(Vec::new()));
    let model = QueueModel {
        responses: RefCell::new([
            r#"{"type":"tool_call","tool":"risky","args":{"token":"secret-value","text":"abcdefghijklmnopqrstuvwxyz"}}"#,
            r#"{"type":"final","content":"finished"}"#,
        ]
        .into()),
        requests: Rc::clone(&requests),
    };
    let runtime = Runtime::new(AgentRegistry::new(vec![Box::new(LocalAgent::new(
        definition,
        Box::new(model),
    ))]))
    .with_tools(ToolRegistry::new(vec![Box::new(RiskyTool { executions })]));
    (runtime, requests)
}

#[test]
fn permission_matrix_matches_the_three_modes() {
    for risk in [
        ToolRisk::ReadOnly,
        ToolRisk::ModifyWorkspace,
        ToolRisk::Destructive,
        ToolRisk::Launch,
        ToolRisk::Command,
        ToolRisk::ReadOutsideWorkspace,
        ToolRisk::ComputerObserve,
        ToolRisk::ComputerControl,
    ] {
        assert!(!requires_confirmation(PermissionMode::FullAccess, risk));
    }
    assert!(!requires_confirmation(
        PermissionMode::Ask,
        ToolRisk::ReadOnly
    ));
    for risk in [
        ToolRisk::ModifyWorkspace,
        ToolRisk::Destructive,
        ToolRisk::Launch,
        ToolRisk::Command,
        ToolRisk::ReadOutsideWorkspace,
        ToolRisk::ComputerObserve,
        ToolRisk::ComputerControl,
    ] {
        assert!(requires_confirmation(PermissionMode::Ask, risk));
    }
    for risk in [ToolRisk::ReadOnly, ToolRisk::ModifyWorkspace] {
        assert!(!requires_confirmation(PermissionMode::Workspace, risk));
    }
    for risk in [
        ToolRisk::Destructive,
        ToolRisk::Launch,
        ToolRisk::Command,
        ToolRisk::ReadOutsideWorkspace,
        ToolRisk::ComputerObserve,
        ToolRisk::ComputerControl,
    ] {
        assert!(requires_confirmation(PermissionMode::Workspace, risk));
    }

    assert_eq!(
        ToolRisk::ReadOutsideWorkspace.reason(),
        "reads files outside the selected workspace and may send sensitive content to the configured model provider"
    );
    assert_eq!(
        ToolRisk::ComputerObserve.reason(),
        "reads potentially sensitive desktop information that may be sent to the configured model provider"
    );
    assert_eq!(
        ToolRisk::ComputerControl.reason(),
        "sends mouse or keyboard input to the active application"
    );
}

#[test]
fn argument_summary_redacts_secrets_and_is_bounded() {
    let summary = summarize_arguments(&serde_json::json!({
        "apiKey": "top-secret",
        "password": "also-secret",
        "path": "notes/file.txt",
        "content": "x".repeat(2_000)
    }));

    assert!(!summary.contains("top-secret"));
    assert!(!summary.contains("also-secret"));
    assert!(summary.contains("[redacted]"));
    assert!(summary.contains("notes/file.txt"));
    assert!(summary.chars().count() <= 512);
}

#[test]
fn runtime_asks_once_and_executes_after_allow() {
    let executions = Rc::new(Cell::new(0));
    let (runtime, _) = test_runtime(Rc::clone(&executions));
    let authorizer = FixedAuthorizer {
        decision: Ok(AuthorizationDecision::Allow),
        calls: Cell::new(0),
    };
    let mut events = Vec::new();

    let answer = runtime
        .ask_stream_with_history_and_authorization(
            "change it",
            &[],
            PermissionMode::Ask,
            &authorizer,
            &mut |_| {},
            &mut |event| events.push(event.clone()),
        )
        .unwrap();

    assert_eq!(answer, "finished");
    assert_eq!(authorizer.calls.get(), 1);
    assert_eq!(executions.get(), 1);
    assert!(matches!(events[0], ToolEvent::Status { .. }));
    assert!(matches!(events[1], ToolEvent::Requested { .. }));
    assert!(matches!(events[2], ToolEvent::ApprovalRequested { .. }));
    assert!(matches!(events[3], ToolEvent::Started { .. }));
    assert!(matches!(
        events[4],
        ToolEvent::Finished { success: true, .. }
    ));
}

#[test]
fn runtime_returns_denial_to_model_without_execution() {
    let executions = Rc::new(Cell::new(0));
    let (runtime, requests) = test_runtime(Rc::clone(&executions));
    let authorizer = FixedAuthorizer {
        decision: Ok(AuthorizationDecision::Deny),
        calls: Cell::new(0),
    };

    let answer = runtime
        .ask_stream_with_history_and_authorization(
            "change it",
            &[],
            PermissionMode::Ask,
            &authorizer,
            &mut |_| {},
            &mut |_| {},
        )
        .unwrap();

    assert_eq!(answer, "finished");
    assert_eq!(authorizer.calls.get(), 1);
    assert_eq!(executions.get(), 0);
    let continuation: serde_json::Value = serde_json::from_str(&requests.borrow()[1]).unwrap();
    assert_eq!(continuation["tool_results"][0]["success"], false);
    assert_eq!(
        continuation["tool_results"][0]["args"],
        serde_json::json!({"token":"secret-value","text":"abcdefghijklmnopqrstuvwxyz"})
    );
    assert_eq!(
        continuation["tool_results"][0]["output"],
        "Tool risky was denied by the user."
    );
}

#[test]
fn denied_tool_emits_denied_without_started() {
    let executions = Rc::new(Cell::new(0));
    let (runtime, _) = test_runtime(Rc::clone(&executions));
    let authorizer = FixedAuthorizer {
        decision: Ok(AuthorizationDecision::Deny),
        calls: Cell::new(0),
    };
    let mut events = Vec::new();

    runtime
        .ask_stream_with_history_and_authorization(
            "change it",
            &[],
            PermissionMode::Ask,
            &authorizer,
            &mut |_| {},
            &mut |event| events.push(event.clone()),
        )
        .unwrap();

    assert!(events.iter().any(|event| matches!(
        event,
        ToolEvent::Denied {
            sequence: 0,
            tool,
            ..
        } if tool == "risky"
    )));
    assert!(!events
        .iter()
        .any(|event| matches!(event, ToolEvent::Started { .. })));
    assert_eq!(executions.get(), 0);
}

#[test]
fn full_access_bypasses_authorizer_and_authorizer_errors_fail_the_turn() {
    let executions = Rc::new(Cell::new(0));
    let (runtime, _) = test_runtime(Rc::clone(&executions));
    let unused = FixedAuthorizer {
        decision: Err("approval transport failed"),
        calls: Cell::new(0),
    };
    runtime
        .ask_stream_with_history_and_authorization(
            "change it",
            &[],
            PermissionMode::FullAccess,
            &unused,
            &mut |_| {},
            &mut |_| {},
        )
        .unwrap();
    assert_eq!(unused.calls.get(), 0);
    assert_eq!(executions.get(), 1);

    let executions = Rc::new(Cell::new(0));
    let (runtime, _) = test_runtime(Rc::clone(&executions));
    let failing = FixedAuthorizer {
        decision: Err("approval transport failed"),
        calls: Cell::new(0),
    };
    let error = runtime
        .ask_stream_with_history_and_authorization(
            "change it",
            &[],
            PermissionMode::Ask,
            &failing,
            &mut |_| {},
            &mut |_| {},
        )
        .unwrap_err();
    assert_eq!(error, "approval transport failed");
    assert_eq!(executions.get(), 0);
}
