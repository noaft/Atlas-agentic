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
    tools::{CreateFileTool, ListDirTool, PwdTool, Tool, ToolDefinition, ToolRegistry, ToolResult},
    Runtime,
};
use chrono::{TimeZone, Utc};

#[cfg(unix)]
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
    runtime.ask_stream(prompt, &mut |_| {})
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

#[cfg(unix)]
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
fn create_file_rejects_parent_directory_paths() {
    let workspace = tempfile::tempdir().unwrap();
    let model = QueueModel::new([r#"
{
  "type": "tool_call",
  "tool": "create_file",
  "args": {
    "path": "../outside.txt",
    "content": "bad"
  }
}
"#]);
    let runtime = Runtime::new(AgentRegistry::new(vec![Box::new(LocalAgent::new(
        definition(),
        Box::new(model),
    ))]))
    .with_tools(ToolRegistry::new(vec![Box::new(CreateFileTool::new(
        workspace.path(),
    ))]));

    let error = ask(&runtime, "create unsafe file").unwrap_err();

    assert_eq!(
        error,
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
    #[cfg(unix)]
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

    #[cfg(unix)]
    assert_eq!(result, "create_file,exec_command,ls,pwd tools available");
    #[cfg(not(unix))]
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
fn ls_rejects_parent_directory_paths() {
    let workspace = tempfile::tempdir().unwrap();
    let model = QueueModel::new([r#"
{
  "type": "tool_call",
  "tool": "ls",
  "args": {
    "path": "../"
  }
}
"#]);
    let runtime = Runtime::new(AgentRegistry::new(vec![Box::new(LocalAgent::new(
        definition(),
        Box::new(model),
    ))]))
    .with_tools(ToolRegistry::new(vec![Box::new(ListDirTool::new(
        workspace.path(),
    ))]));

    let error = ask(&runtime, "list outside").unwrap_err();

    assert_eq!(error, "ls path cannot contain parent directory segments");
}
