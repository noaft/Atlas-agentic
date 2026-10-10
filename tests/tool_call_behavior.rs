use std::{
    cell::{Cell, RefCell},
    collections::VecDeque,
    fs,
    rc::Rc,
};

use atlas_agentic::{
    agent_output::AgentOutput,
    agents::{AgentRegistry, LocalAgent, LocalAgentDefinition},
    conversations::{ConversationMessage, MessageRole},
    models::{ModelProvider, ModelRequest},
    permissions::{AllowAllAuthorizer, PermissionMode, ToolEvent},
    tools::{
        CreateFileTool, CurrentTimeTool, ListDirTool, PwdTool, ReadFileTool, Tool, ToolDefinition,
        ToolRegistry, ToolResult,
    },
    Runtime,
};
use chrono::{DateTime, Local, TimeZone, Utc};

#[cfg(any(unix, windows))]
use atlas_agentic::tools::ExecCommandTool;

struct QueueModel {
    responses: RefCell<VecDeque<String>>,
}

impl QueueModel {
    fn new(responses: impl IntoIterator<Item = impl Into<String>>) -> Self {
        Self {
            responses: RefCell::new(responses.into_iter().map(Into::into).collect()),
        }
    }
}

impl ModelProvider for QueueModel {
    fn complete(&self, _request: &ModelRequest) -> Result<String, String> {
        self.responses
            .borrow_mut()
            .pop_front()
            .ok_or_else(|| "no fake response queued".to_string())
    }
}

struct ToolAwareModel;

impl ModelProvider for ToolAwareModel {
    fn complete(&self, request: &ModelRequest) -> Result<String, String> {
        let mut tool_names = request
            .tools
            .iter()
            .map(|tool| tool.name.as_str())
            .collect::<Vec<_>>();
        tool_names.sort();

        Ok(format!(
            r#"{{"type":"final","content":"{} tools available"}}"#,
            tool_names.join(",")
        ))
    }
}

struct StreamingModel;

impl ModelProvider for StreamingModel {
    fn complete(&self, _request: &ModelRequest) -> Result<String, String> {
        Ok("hello world".to_string())
    }

    fn stream(
        &self,
        _request: &ModelRequest,
        on_delta: &mut dyn FnMut(&str),
    ) -> Result<String, String> {
        on_delta("hello");
        on_delta(" ");
        on_delta("world");
        Ok("hello world".to_string())
    }
}

struct ToolProtocolStreamingModel {
    responses: RefCell<VecDeque<Vec<&'static str>>>,
}

#[derive(Debug, PartialEq, Eq)]
struct CapturedHistoryRequest {
    history: Vec<ConversationMessage>,
    user_prompt: String,
}

struct HistoryCapturingStreamingModel {
    responses: RefCell<VecDeque<Vec<&'static str>>>,
    requests: Rc<RefCell<Vec<CapturedHistoryRequest>>>,
}

impl HistoryCapturingStreamingModel {
    fn new(
        responses: impl IntoIterator<Item = Vec<&'static str>>,
        requests: Rc<RefCell<Vec<CapturedHistoryRequest>>>,
    ) -> Self {
        Self {
            responses: RefCell::new(responses.into_iter().collect()),
            requests,
        }
    }
}

impl ModelProvider for HistoryCapturingStreamingModel {
    fn complete(&self, _request: &ModelRequest) -> Result<String, String> {
        Err("history capturing model requires stream".to_string())
    }

    fn stream(
        &self,
        request: &ModelRequest,
        on_delta: &mut dyn FnMut(&str),
    ) -> Result<String, String> {
        self.requests.borrow_mut().push(CapturedHistoryRequest {
            history: request.history.to_vec(),
            user_prompt: request.user_prompt.to_string(),
        });
        let chunks = self
            .responses
            .borrow_mut()
            .pop_front()
            .ok_or_else(|| "no fake streaming response queued".to_string())?;
        let mut response = String::new();
        for chunk in chunks {
            on_delta(chunk);
            response.push_str(chunk);
        }
        Ok(response)
    }
}

impl ToolProtocolStreamingModel {
    fn new(responses: impl IntoIterator<Item = Vec<&'static str>>) -> Self {
        Self {
            responses: RefCell::new(responses.into_iter().collect()),
        }
    }
}

impl ModelProvider for ToolProtocolStreamingModel {
    fn complete(&self, _request: &ModelRequest) -> Result<String, String> {
        Err("tool protocol streaming model requires stream".to_string())
    }

    fn stream(
        &self,
        _request: &ModelRequest,
        on_delta: &mut dyn FnMut(&str),
    ) -> Result<String, String> {
        let chunks = self
            .responses
            .borrow_mut()
            .pop_front()
            .ok_or_else(|| "no fake streaming response queued".to_string())?;
        let mut response = String::new();
        for chunk in chunks {
            on_delta(chunk);
            response.push_str(chunk);
        }
        Ok(response)
    }
}

struct SamplingAwareModel;

impl ModelProvider for SamplingAwareModel {
    fn complete(&self, request: &ModelRequest) -> Result<String, String> {
        Ok(format!(
            "temperature={:?}; top_p={:?}",
            request.temperature, request.top_p
        ))
    }
}

struct ToolResultAwareModel;

impl ModelProvider for ToolResultAwareModel {
    fn complete(&self, request: &ModelRequest) -> Result<String, String> {
        if request.user_prompt == "where am I?" {
            return Ok(r#"{"type":"tool_call","tool":"pwd","args":{}}"#.to_string());
        }

        let message: serde_json::Value = serde_json::from_str(request.user_prompt)
            .map_err(|error| format!("tool continuation was not valid JSON: {error}"))?;
        let result = &message["tool_results"][0];
        if message["type"] == "tool_results"
            && message["original_user_request"] == "where am I?"
            && message["tool_results"].as_array().map(Vec::len) == Some(1)
            && result["tool"] == "pwd"
            && result["args"] == serde_json::json!({})
            && result["output"]
                .as_str()
                .is_some_and(|output| output.starts_with("Workspace: "))
        {
            return Ok(
                r#"{"type":"final","content":"Atlas confirmed the workspace."}"#.to_string(),
            );
        }

        Err(format!(
            "agent did not receive recognizable tool output: {}",
            request.user_prompt
        ))
    }
}

struct SequentialToolModel;

impl ModelProvider for SequentialToolModel {
    fn complete(&self, request: &ModelRequest) -> Result<String, String> {
        if request.user_prompt == "inspect the workspace" {
            return Ok(r#"{"type":"tool_call","tool":"pwd","args":{}}"#.to_string());
        }

        let message: serde_json::Value = serde_json::from_str(request.user_prompt)
            .map_err(|error| format!("tool continuation was not valid JSON: {error}"))?;
        if message["type"] != "tool_results"
            || message["original_user_request"] != "inspect the workspace"
        {
            return Err(format!("invalid tool continuation: {message}"));
        }
        let results = message["tool_results"]
            .as_array()
            .ok_or_else(|| format!("tool_results was not an array: {message}"))?;

        if results.len() == 1
            && results[0]["tool"] == "pwd"
            && results[0]["args"] == serde_json::json!({})
            && results[0]["output"]
                .as_str()
                .is_some_and(|output| output.starts_with("Workspace: "))
        {
            return Ok(r#"{"type":"tool_call","tool":"ls","args":{}}"#.to_string());
        }
        if results.len() == 2
            && results[0]["tool"] == "pwd"
            && results[1]["tool"] == "ls"
            && results[1]["args"] == serde_json::json!({})
            && results[1]["output"] == "note.txt"
        {
            return Ok(
                r#"{"type":"final","content":"Atlas inspected the workspace."}"#.to_string(),
            );
        }

        Err(format!(
            "agent received unexpected tool history: {}",
            request.user_prompt
        ))
    }
}

struct RepeatingToolModel;

impl ModelProvider for RepeatingToolModel {
    fn complete(&self, _request: &ModelRequest) -> Result<String, String> {
        Ok(r#"{"type":"tool_call","tool":"pwd","args":{}}"#.to_string())
    }
}

struct RepeatingMissingToolModel;

impl ModelProvider for RepeatingMissingToolModel {
    fn complete(&self, _request: &ModelRequest) -> Result<String, String> {
        Ok(r#"{"type":"tool_call","tool":"missing","args":{}}"#.to_string())
    }
}

struct InvalidProtocolModel {
    response: &'static str,
}

impl ModelProvider for InvalidProtocolModel {
    fn complete(&self, _request: &ModelRequest) -> Result<String, String> {
        Ok(self.response.to_string())
    }
}

struct CountingTool {
    executions: Rc<Cell<usize>>,
}

struct FailingTool;

impl Tool for FailingTool {
    fn name(&self) -> &str {
        "failing"
    }

    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: self.name().to_string(),
            description: "Always fail for runtime recovery tests.".to_string(),
            input_schema: serde_json::json!({"type": "object"}),
        }
    }

    fn execute(&self, _args: &serde_json::Value) -> Result<ToolResult, String> {
        Err("simulated tool failure".to_string())
    }
}

impl Tool for CountingTool {
    fn name(&self) -> &str {
        "pwd"
    }

    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: self.name().to_string(),
            description: "Count executions.".to_string(),
            input_schema: serde_json::json!({"type": "object"}),
        }
    }

    fn execute(&self, _args: &serde_json::Value) -> Result<ToolResult, String> {
        self.executions.set(self.executions.get() + 1);
        Ok(ToolResult {
            message: "counted".to_string(),
        })
    }
}

fn definition() -> LocalAgentDefinition {
    LocalAgentDefinition::from_toml(
        r#"
name = "atlas"
display_name = "Atlas"
model_provider = "fake"
model = "fake"
system_prompt = "You are Atlas."
conversation_title_prompt = "Write a short conversation title."
"#,
    )
    .unwrap()
}

fn ask(runtime: &Runtime, prompt: &str) -> Result<String, String> {
    runtime.ask_stream_with_history_and_authorization(
        prompt,
        &[],
        PermissionMode::FullAccess,
        &AllowAllAuthorizer,
        &mut |_| {},
        &mut |_| {},
    )
}

#[test]
fn parses_final_agent_output() {
    let output = AgentOutput::parse(
        r#"
{
  "type": "final",
  "content": "Hello."
}
"#,
    )
    .unwrap();

    assert_eq!(
        output,
        AgentOutput::Final {
            content: "Hello.".to_string()
        }
    );
}

#[test]
fn tool_protocol_requires_type_as_the_first_field() {
    let error =
        AgentOutput::parse_tool_protocol(r#"{"content":"Hello.","type":"final"}"#).unwrap_err();

    assert_eq!(
        error,
        "invalid agent tool protocol: type must be the first field"
    );
}

#[test]
fn treats_plain_text_as_final_agent_output() {
    let output = AgentOutput::parse("plain answer").unwrap();

    assert_eq!(
        output,
        AgentOutput::Final {
            content: "plain answer".to_string()
        }
    );
}

#[test]
fn runtime_executes_create_file_tool_call() {
    let workspace = tempfile::tempdir().unwrap();
    let model = QueueModel::new([
        r#"
{
  "type": "tool_call",
  "tool": "create_file",
  "args": {
    "path": "notes/hello.txt",
    "content": "Hello from Atlas"
  }
}
"#,
        r#"{"type":"final","content":"The file is ready."}"#,
    ]);
    let runtime = Runtime::new(AgentRegistry::new(vec![Box::new(LocalAgent::new(
        definition(),
        Box::new(model),
    ))]))
    .with_tools(ToolRegistry::new(vec![Box::new(CreateFileTool::new(
        workspace.path(),
    ))]));

    let result = ask(&runtime, "create a file").unwrap();

    assert_eq!(result, "The file is ready.");
    assert_eq!(
        fs::read_to_string(workspace.path().join("notes/hello.txt")).unwrap(),
        "Hello from Atlas"
    );
}

#[cfg(any(unix, windows))]
#[test]
fn runtime_executes_command_tool_call_and_returns_agent_answer() {
    let workspace = tempfile::tempdir().unwrap();
    let model = QueueModel::new([
        r#"{"type":"tool_call","tool":"exec_command","args":{"command":"echo executed > exec-output.txt"}}"#,
        r#"{"type":"final","content":"The command completed."}"#,
    ]);
    let runtime = Runtime::new(AgentRegistry::new(vec![Box::new(LocalAgent::new(
        definition(),
        Box::new(model),
    ))]))
    .with_tools(ToolRegistry::new(vec![Box::new(ExecCommandTool::new(
        workspace.path(),
    ))]));

    let result = ask(&runtime, "run a command").unwrap();

    assert_eq!(result, "The command completed.");
    assert_eq!(
        fs::read_to_string(workspace.path().join("exec-output.txt"))
            .unwrap()
            .trim(),
        "executed"
    );
}

#[test]
fn runtime_returns_create_file_validation_failure_to_agent() {
    let workspace = tempfile::tempdir().unwrap();
    let requests = Rc::new(RefCell::new(Vec::new()));
    let model = HistoryCapturingStreamingModel::new(
        [
            vec![
                r#"
{
  "type": "tool_call",
  "tool": "create_file",
  "args": {
    "path": "../outside.txt",
    "content": "bad"
  }
}
"#,
            ],
            vec![r#"{"type":"final","content":"Used a safe path instead."}"#],
        ],
        Rc::clone(&requests),
    );
    let runtime = Runtime::new(AgentRegistry::new(vec![Box::new(LocalAgent::new(
        definition(),
        Box::new(model),
    ))]))
    .with_tools(ToolRegistry::new(vec![Box::new(CreateFileTool::new(
        workspace.path(),
    ))]));

    let answer = ask(&runtime, "create unsafe file").unwrap();

    assert_eq!(answer, "Used a safe path instead.");
    let retry: serde_json::Value = serde_json::from_str(&requests.borrow()[1].user_prompt).unwrap();
    assert_eq!(retry["tool_results"][0]["success"], false);
    assert_eq!(
        retry["tool_results"][0]["output"],
        "create_file path cannot contain parent directory segments"
    );
}

#[test]
fn runtime_passes_registered_tools_to_model_request() {
    let workspace = tempfile::tempdir().unwrap();
    let tools = ToolRegistry::new(vec![
        Box::new(CreateFileTool::new(workspace.path())),
        Box::new(PwdTool::new(workspace.path())),
        Box::new(ListDirTool::new(workspace.path())),
    ]);
    #[cfg(any(unix, windows))]
    let tools = {
        let mut tools = tools;
        tools.register(Box::new(ExecCommandTool::new(workspace.path())));
        tools
    };
    let runtime = Runtime::new(AgentRegistry::new(vec![Box::new(LocalAgent::new(
        definition(),
        Box::new(ToolAwareModel),
    ))]))
    .with_tools(tools);

    let result = ask(&runtime, "what tools exist?").unwrap();

    #[cfg(any(unix, windows))]
    assert_eq!(result, "create_file,exec_command,ls,pwd tools available");
    #[cfg(not(any(unix, windows)))]
    assert_eq!(result, "create_file,ls,pwd tools available");
}

#[test]
fn runtime_passes_agent_sampling_settings_to_model_request() {
    let definition = LocalAgentDefinition::from_toml(
        r#"
name = "atlas"
display_name = "Atlas"
model_provider = "fake"
model = "fake"
temperature = 0.2
top_p = 0.7
system_prompt = "You are Atlas."
conversation_title_prompt = "Write a short conversation title."
"#,
    )
    .unwrap();
    let runtime = Runtime::new(AgentRegistry::new(vec![Box::new(LocalAgent::new(
        definition,
        Box::new(SamplingAwareModel),
    ))]));

    let result = ask(&runtime, "sampling?").unwrap();

    assert_eq!(result, "temperature=Some(0.2); top_p=Some(0.7)");
}

#[test]
fn runtime_streams_model_deltas_and_returns_final_message() {
    let runtime = Runtime::new(AgentRegistry::new(vec![Box::new(LocalAgent::new(
        definition(),
        Box::new(StreamingModel),
    ))]));
    let mut deltas = Vec::new();

    let result = runtime
        .ask_stream("hello", &mut |delta| deltas.push(delta.to_string()))
        .unwrap();

    assert_eq!(deltas, vec!["hello", " ", "world"]);
    assert_eq!(result, "hello world");
}

#[test]
fn runtime_executes_pwd_tool_call() {
    let workspace = tempfile::tempdir().unwrap();
    let model = QueueModel::new([
        r#"
{
  "type": "tool_call",
  "tool": "pwd",
  "args": {}
}
"#,
        r#"{"type":"final","content":"The workspace was found."}"#,
    ]);
    let runtime = Runtime::new(AgentRegistry::new(vec![Box::new(LocalAgent::new(
        definition(),
        Box::new(model),
    ))]))
    .with_tools(ToolRegistry::new(vec![Box::new(PwdTool::new(
        workspace.path(),
    ))]));

    let result = ask(&runtime, "where am I?").unwrap();

    assert_eq!(result, "The workspace was found.");
}

#[test]
fn current_time_tool_reports_the_machine_local_date_time() {
    let before = Local::now().fixed_offset();

    let result = CurrentTimeTool::new()
        .execute(&serde_json::json!({}))
        .unwrap();

    let after = Local::now().fixed_offset();
    let value = result
        .message
        .strip_prefix("Local date and time: ")
        .expect("current time output needs a clear label");
    let reported = DateTime::parse_from_rfc3339(value).expect("current time must be RFC 3339");
    assert!(reported >= before, "reported {reported} before {before}");
    assert!(reported <= after, "reported {reported} after {after}");
    assert_eq!(
        reported.offset().local_minus_utc(),
        before.offset().local_minus_utc()
    );
}

#[test]
fn runtime_sends_tool_output_back_to_agent_before_returning_final_answer() {
    let workspace = tempfile::tempdir().unwrap();
    let runtime = Runtime::new(AgentRegistry::new(vec![Box::new(LocalAgent::new(
        definition(),
        Box::new(ToolResultAwareModel),
    ))]))
    .with_tools(ToolRegistry::new(vec![Box::new(PwdTool::new(
        workspace.path(),
    ))]));

    let result = ask(&runtime, "where am I?").unwrap();

    assert_eq!(result, "Atlas confirmed the workspace.");
}

#[test]
fn runtime_returns_unknown_tool_failure_to_agent_and_accepts_corrected_retry() {
    let workspace = tempfile::tempdir().unwrap();
    let requests = Rc::new(RefCell::new(Vec::new()));
    let model = HistoryCapturingStreamingModel::new(
        [
            vec![r#"{"type":"tool_call","tool":"missing","args":{}}"#],
            vec![r#"{"type":"tool_call","tool":"pwd","args":{}}"#],
            vec![r#"{"type":"final","content":"Recovered after retry."}"#],
        ],
        Rc::clone(&requests),
    );
    let runtime = Runtime::new(AgentRegistry::new(vec![Box::new(LocalAgent::new(
        definition(),
        Box::new(model),
    ))]))
    .with_tools(ToolRegistry::new(vec![Box::new(PwdTool::new(
        workspace.path(),
    ))]));

    let answer = ask(&runtime, "find the workspace").unwrap();

    assert_eq!(answer, "Recovered after retry.");
    let requests = requests.borrow();
    assert_eq!(requests.len(), 3);
    let first_retry: serde_json::Value = serde_json::from_str(&requests[1].user_prompt).unwrap();
    assert_eq!(first_retry["tool_results"][0]["tool"], "missing");
    assert_eq!(first_retry["tool_results"][0]["success"], false);
    assert_eq!(
        first_retry["tool_results"][0]["output"],
        "unknown tool: missing"
    );
    let corrected_retry: serde_json::Value =
        serde_json::from_str(&requests[2].user_prompt).unwrap();
    assert_eq!(corrected_retry["tool_results"].as_array().unwrap().len(), 2);
    assert_eq!(corrected_retry["tool_results"][1]["tool"], "pwd");
    assert_eq!(corrected_retry["tool_results"][1]["success"], true);
}

#[test]
fn runtime_with_no_registered_tools_returns_lookup_failure_to_agent() {
    let requests = Rc::new(RefCell::new(Vec::new()));
    let model = HistoryCapturingStreamingModel::new(
        [
            vec![r#"{"type":"tool_call","tool":"missing","args":{}}"#],
            vec!["Recovered ", "without tools."],
        ],
        Rc::clone(&requests),
    );
    let runtime = Runtime::new(AgentRegistry::new(vec![Box::new(LocalAgent::new(
        definition(),
        Box::new(model),
    ))]));
    let mut deltas = Vec::new();

    let answer = runtime
        .ask_stream("try a tool", &mut |delta| deltas.push(delta.to_string()))
        .unwrap();

    assert_eq!(answer, "Recovered without tools.");
    assert_eq!(deltas, vec!["Recovered ", "without tools."]);
    let requests = requests.borrow();
    assert_eq!(requests.len(), 2);
    let retry: serde_json::Value = serde_json::from_str(&requests[1].user_prompt).unwrap();
    assert_eq!(retry["tool_results"][0]["tool"], "missing");
    assert_eq!(retry["tool_results"][0]["success"], false);
    assert_eq!(retry["tool_results"][0]["output"], "unknown tool: missing");
}

#[test]
fn runtime_returns_execution_failure_to_agent_and_emits_failed_event() {
    let requests = Rc::new(RefCell::new(Vec::new()));
    let model = HistoryCapturingStreamingModel::new(
        [
            vec![r#"{"type":"tool_call","tool":"failing","args":{}}"#],
            vec![r#"{"type":"final","content":"Handled the failure."}"#],
        ],
        Rc::clone(&requests),
    );
    let runtime = Runtime::new(AgentRegistry::new(vec![Box::new(LocalAgent::new(
        definition(),
        Box::new(model),
    ))]))
    .with_tools(ToolRegistry::new(vec![Box::new(FailingTool)]));
    let mut events = Vec::new();

    let answer = runtime
        .ask_stream_with_history_and_authorization(
            "run it",
            &[],
            PermissionMode::FullAccess,
            &AllowAllAuthorizer,
            &mut |_| {},
            &mut |event| events.push(event.clone()),
        )
        .unwrap();

    assert_eq!(answer, "Handled the failure.");
    let retry: serde_json::Value = serde_json::from_str(&requests.borrow()[1].user_prompt).unwrap();
    assert_eq!(retry["tool_results"][0]["success"], false);
    assert_eq!(retry["tool_results"][0]["output"], "simulated tool failure");
    assert!(matches!(events[0], ToolEvent::Started { .. }));
    assert!(matches!(
        events[1],
        ToolEvent::Finished { success: false, .. }
    ));
}

#[test]
fn runtime_stream_hides_tool_protocol_and_emits_agent_final_answer() {
    let workspace = tempfile::tempdir().unwrap();
    let runtime = Runtime::new(AgentRegistry::new(vec![Box::new(LocalAgent::new(
        definition(),
        Box::new(ToolResultAwareModel),
    ))]))
    .with_tools(ToolRegistry::new(vec![Box::new(PwdTool::new(
        workspace.path(),
    ))]));
    let mut deltas = Vec::new();

    let result = runtime
        .ask_stream("where am I?", &mut |delta| deltas.push(delta.to_string()))
        .unwrap();

    assert_eq!(deltas, vec!["Atlas confirmed the workspace."]);
    assert_eq!(result, "Atlas confirmed the workspace.");
}

#[test]
fn runtime_streams_final_content_incrementally_after_a_tool_call() {
    let workspace = tempfile::tempdir().unwrap();
    let model = ToolProtocolStreamingModel::new([
        vec![r#"{"type":"tool_call","tool":"pwd","args":{}}"#],
        vec![
            r#"{"type":"final","content":"Atlas "#,
            "streams ",
            r#"now."}"#,
        ],
    ]);
    let runtime = Runtime::new(AgentRegistry::new(vec![Box::new(LocalAgent::new(
        definition(),
        Box::new(model),
    ))]))
    .with_tools(ToolRegistry::new(vec![Box::new(PwdTool::new(
        workspace.path(),
    ))]));
    let mut deltas = Vec::new();

    let result = runtime
        .ask_stream("where am I?", &mut |delta| deltas.push(delta.to_string()))
        .unwrap();

    assert_eq!(deltas, vec!["Atlas ", "streams ", "now."]);
    assert_eq!(result, "Atlas streams now.");
}

#[test]
fn runtime_reuses_saved_history_across_tool_calls_and_streams_the_final_answer() {
    let workspace = tempfile::tempdir().unwrap();
    let requests = Rc::new(RefCell::new(Vec::new()));
    let model = HistoryCapturingStreamingModel::new(
        [
            vec![r#"{"type":"tool_call","tool":"pwd","args":{}}"#],
            vec![
                r#"{"type":"final","content":"Atlas "#,
                "streams ",
                r#"with history."}"#,
            ],
        ],
        Rc::clone(&requests),
    );
    let runtime = Runtime::new(AgentRegistry::new(vec![Box::new(LocalAgent::new(
        definition(),
        Box::new(model),
    ))]))
    .with_tools(ToolRegistry::new(vec![Box::new(PwdTool::new(
        workspace.path(),
    ))]));
    let history = vec![
        ConversationMessage {
            role: MessageRole::User,
            content: "Earlier question".to_string(),
            created_at: Utc.with_ymd_and_hms(2026, 10, 4, 12, 0, 0).unwrap(),
        },
        ConversationMessage {
            role: MessageRole::Assistant,
            content: "Earlier answer".to_string(),
            created_at: Utc.with_ymd_and_hms(2026, 10, 4, 12, 0, 1).unwrap(),
        },
    ];
    let mut deltas = Vec::new();

    let result = runtime
        .ask_stream_with_history("where am I?", &history, &mut |delta| {
            deltas.push(delta.to_string())
        })
        .unwrap();

    assert_eq!(result, "Atlas streams with history.");
    assert_eq!(deltas, vec!["Atlas ", "streams ", "with history."]);
    let requests = requests.borrow();
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[0].history, history);
    assert_eq!(requests[1].history, history);
    assert_eq!(requests[0].user_prompt, "where am I?");
    assert!(requests[1]
        .user_prompt
        .contains(r#""type": "tool_results""#));
    assert!(requests[1]
        .user_prompt
        .contains(r#""original_user_request": "where am I?""#));
    assert!(!requests[1].user_prompt.contains("Earlier question"));
    assert!(!requests[1].user_prompt.contains("Earlier answer"));
}

#[test]
fn runtime_stream_rejects_invalid_tool_protocol_without_emitting_it() {
    for response in [
        r#"{"type":"tool_call","tool":"pwd""#,
        r#"{"type":"tool_call","args":{}}"#,
        r#"{"content":"must stay hidden","type":"final"}"#,
    ] {
        let workspace = tempfile::tempdir().unwrap();
        let runtime = Runtime::new(AgentRegistry::new(vec![Box::new(LocalAgent::new(
            definition(),
            Box::new(InvalidProtocolModel { response }),
        ))]))
        .with_tools(ToolRegistry::new(vec![Box::new(PwdTool::new(
            workspace.path(),
        ))]));
        let mut deltas = Vec::new();

        let error = runtime
            .ask_stream("where am I?", &mut |delta| deltas.push(delta.to_string()))
            .unwrap_err();

        assert!(error.starts_with("invalid agent tool protocol:"), "{error}");
        assert!(deltas.is_empty(), "leaked protocol: {deltas:?}");
    }
}

#[test]
fn runtime_retries_one_non_json_tool_response_and_preserves_the_pending_request() {
    let requests = Rc::new(RefCell::new(Vec::new()));
    let model = HistoryCapturingStreamingModel::new(
        [
            vec!["The file is in the workspace."],
            vec![r#"{"type":"final","content":"The file is in the workspace."}"#],
        ],
        Rc::clone(&requests),
    );
    let runtime = Runtime::new(AgentRegistry::new(vec![Box::new(LocalAgent::new(
        definition(),
        Box::new(model),
    ))]))
    .with_tools(ToolRegistry::new(vec![Box::new(PwdTool::new("."))]));
    let mut deltas = Vec::new();

    let result = runtime
        .ask_stream("where is this file?", &mut |delta| {
            deltas.push(delta.to_string())
        })
        .unwrap();

    assert_eq!(result, "The file is in the workspace.");
    assert_eq!(deltas, vec!["The file is in the workspace."]);
    let requests = requests.borrow();
    assert_eq!(requests.len(), 2);
    let repair: serde_json::Value = serde_json::from_str(&requests[1].user_prompt).unwrap();
    assert_eq!(repair["type"], "protocol_repair");
    assert_eq!(repair["pending_request"], "where is this file?");
}

#[test]
fn runtime_retries_one_empty_tool_response_and_preserves_the_pending_request() {
    let workspace = tempfile::tempdir().unwrap();
    fs::write(
        workspace.path().join("note.txt"),
        "hello from the workspace",
    )
    .unwrap();
    let requests = Rc::new(RefCell::new(Vec::new()));
    let model = HistoryCapturingStreamingModel::new(
        [
            vec![""],
            vec![r#"{"type":"tool_call","tool":"read_file","args":{"path":"note.txt"}}"#],
            vec![r#"{"type":"final","content":"Atlas read the file."}"#],
        ],
        Rc::clone(&requests),
    );
    let runtime = Runtime::new(AgentRegistry::new(vec![Box::new(LocalAgent::new(
        definition(),
        Box::new(model),
    ))]))
    .with_tools(ToolRegistry::new(vec![Box::new(ReadFileTool::new(
        workspace.path(),
    ))]));
    let mut deltas = Vec::new();

    let result = runtime
        .ask_stream("read note.txt", &mut |delta| deltas.push(delta.to_string()))
        .unwrap();

    assert_eq!(result, "Atlas read the file.");
    assert_eq!(deltas, vec!["Atlas read the file."]);
    let requests = requests.borrow();
    assert_eq!(requests.len(), 3);
    let repair: serde_json::Value = serde_json::from_str(&requests[1].user_prompt).unwrap();
    assert_eq!(repair["type"], "protocol_repair");
    assert_eq!(repair["pending_request"], "read note.txt");
    let continuation: serde_json::Value = serde_json::from_str(&requests[2].user_prompt).unwrap();
    assert_eq!(continuation["type"], "tool_results");
    assert_eq!(continuation["tool_results"][0]["tool"], "read_file");
    assert_eq!(
        continuation["tool_results"][0]["output"],
        "hello from the workspace"
    );
}

#[test]
fn runtime_reports_a_second_consecutive_empty_tool_response() {
    let workspace = tempfile::tempdir().unwrap();
    let model = ToolProtocolStreamingModel::new([vec![""], vec![""]]);
    let runtime = Runtime::new(AgentRegistry::new(vec![Box::new(LocalAgent::new(
        definition(),
        Box::new(model),
    ))]))
    .with_tools(ToolRegistry::new(vec![Box::new(ReadFileTool::new(
        workspace.path(),
    ))]));

    let error = ask(&runtime, "read note.txt").unwrap_err();

    assert_eq!(error, "invalid agent tool protocol: response was empty");
}

#[test]
fn runtime_sends_accumulated_tool_outputs_back_to_agent() {
    let workspace = tempfile::tempdir().unwrap();
    fs::write(workspace.path().join("note.txt"), "hello").unwrap();
    let runtime = Runtime::new(AgentRegistry::new(vec![Box::new(LocalAgent::new(
        definition(),
        Box::new(SequentialToolModel),
    ))]))
    .with_tools(ToolRegistry::new(vec![
        Box::new(PwdTool::new(workspace.path())),
        Box::new(ListDirTool::new(workspace.path())),
    ]));

    let result = ask(&runtime, "inspect the workspace").unwrap();

    assert_eq!(result, "Atlas inspected the workspace.");
}

#[test]
fn runtime_stops_an_agent_that_exceeds_the_tool_call_limit() {
    let executions = Rc::new(Cell::new(0));
    let runtime = Runtime::new(AgentRegistry::new(vec![Box::new(LocalAgent::new(
        definition(),
        Box::new(RepeatingToolModel),
    ))]))
    .with_tools(ToolRegistry::new(vec![Box::new(CountingTool {
        executions: Rc::clone(&executions),
    })]));

    let error = ask(&runtime, "keep using tools").unwrap_err();

    assert_eq!(error, "agent exceeded the maximum of 8 tool calls");
    assert_eq!(executions.get(), 8);
}

#[test]
fn runtime_counts_repeated_tool_failures_toward_the_tool_call_limit() {
    let runtime = Runtime::new(AgentRegistry::new(vec![Box::new(LocalAgent::new(
        definition(),
        Box::new(RepeatingMissingToolModel),
    ))]))
    .with_tools(ToolRegistry::new(vec![Box::new(PwdTool::new("."))]));

    let error = ask(&runtime, "keep trying a missing tool").unwrap_err();

    assert_eq!(error, "agent exceeded the maximum of 8 tool calls");
}

#[test]
fn runtime_executes_ls_tool_call() {
    let workspace = tempfile::tempdir().unwrap();
    fs::create_dir(workspace.path().join("notes")).unwrap();
    fs::write(workspace.path().join("notes/a.txt"), "a").unwrap();
    fs::write(workspace.path().join("notes/b.txt"), "b").unwrap();
    let model = QueueModel::new([
        r#"
{
  "type": "tool_call",
  "tool": "ls",
  "args": {
    "path": "notes"
  }
}
"#,
        r#"{"type":"final","content":"The notes were listed."}"#,
    ]);
    let runtime = Runtime::new(AgentRegistry::new(vec![Box::new(LocalAgent::new(
        definition(),
        Box::new(model),
    ))]))
    .with_tools(ToolRegistry::new(vec![Box::new(ListDirTool::new(
        workspace.path(),
    ))]));

    let result = ask(&runtime, "list notes").unwrap();

    assert_eq!(result, "The notes were listed.");
}

#[test]
fn runtime_continues_after_ls_rejects_parent_directory_path() {
    let workspace = tempfile::tempdir().unwrap();
    let model = QueueModel::new([
        r#"
{
  "type": "tool_call",
  "tool": "ls",
  "args": {
    "path": "../"
  }
}
"#,
        r#"{"type":"final","content":"Cannot list outside the workspace."}"#,
    ]);
    let runtime = Runtime::new(AgentRegistry::new(vec![Box::new(LocalAgent::new(
        definition(),
        Box::new(model),
    ))]))
    .with_tools(ToolRegistry::new(vec![Box::new(ListDirTool::new(
        workspace.path(),
    ))]));

    let answer = ask(&runtime, "list outside").unwrap();

    assert_eq!(answer, "Cannot list outside the workspace.");
}
