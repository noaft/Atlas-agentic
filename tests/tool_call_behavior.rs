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
    media::{ImageData, ImageDetail},
    models::{ModelProvider, ModelRequest},
    permissions::{AllowAllAuthorizer, PermissionMode, RuntimeStatus, ToolEvent},
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

enum SyntheticToolOutput {
    Image(&'static [u8]),
    OversizedImage(&'static [u8]),
    Text,
    Failure,
}

struct SyntheticTool {
    name: &'static str,
    output: SyntheticToolOutput,
}

impl Tool for SyntheticTool {
    fn name(&self) -> &str {
        self.name
    }

    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: self.name.to_string(),
            description: "Synthetic lifecycle tool.".to_string(),
            input_schema: serde_json::json!({"type": "object"}),
        }
    }

    fn execute(&self, _args: &serde_json::Value) -> Result<ToolResult, String> {
        match self.output {
            SyntheticToolOutput::Image(bytes) => Ok(ToolResult::with_image(
                "Captured synthetic screen.",
                ImageData {
                    mime_type: "image/png".to_string(),
                    bytes: bytes.to_vec(),
                    width: 2,
                    height: 1,
                    detail: ImageDetail::Original,
                },
            )),
            SyntheticToolOutput::OversizedImage(bytes) => Ok(ToolResult::with_image(
                "Captured oversized synthetic screen.",
                ImageData {
                    mime_type: "image/png".to_string(),
                    bytes: bytes.to_vec(),
                    width: 8193,
                    height: 1,
                    detail: ImageDetail::Original,
                },
            )),
            SyntheticToolOutput::Text => Ok(ToolResult::text("text-only result")),
            SyntheticToolOutput::Failure => Err("synthetic capture failed".to_string()),
        }
    }
}

struct ImageLifecycleModel {
    responses: RefCell<VecDeque<String>>,
    images: Rc<RefCell<Vec<Vec<ImageData>>>>,
    prompts: Rc<RefCell<Vec<String>>>,
}

type CapturedImageBatches = Rc<RefCell<Vec<Vec<ImageData>>>>;
type CapturedPrompts = Rc<RefCell<Vec<String>>>;

impl ModelProvider for ImageLifecycleModel {
    fn validate_images(&self, images: &[ImageData]) -> Result<(), String> {
        if images.iter().any(|image| image.width > 8192) {
            Err("synthetic provider rejects images wider than 8192 pixels".to_string())
        } else {
            Ok(())
        }
    }

    fn complete(&self, request: &ModelRequest) -> Result<String, String> {
        self.images.borrow_mut().push(request.images.to_vec());
        self.prompts
            .borrow_mut()
            .push(request.user_prompt.to_string());
        self.responses
            .borrow_mut()
            .pop_front()
            .ok_or_else(|| "no lifecycle response queued".to_string())
    }
}

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
        Ok(ToolResult::text("counted"))
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

fn definition_with_protocol_repair_limit(limit: usize) -> LocalAgentDefinition {
    let mut definition = definition();
    definition.max_protocol_repairs = Some(limit);
    definition
}

fn definition_with_tool_call_limit(max_tool_calls: usize) -> LocalAgentDefinition {
    LocalAgentDefinition::from_toml(&format!(
        r#"
name = "atlas"
display_name = "Atlas"
model_provider = "fake"
model = "fake"
max_tool_calls = {max_tool_calls}
system_prompt = "You are Atlas."
conversation_title_prompt = "Write a short conversation title."
"#
    ))
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

fn image_lifecycle_runtime(
    responses: impl IntoIterator<Item = &'static str>,
    tools: Vec<Box<dyn Tool>>,
) -> (Runtime, CapturedImageBatches, CapturedPrompts) {
    let images = Rc::new(RefCell::new(Vec::new()));
    let prompts = Rc::new(RefCell::new(Vec::new()));
    let model = ImageLifecycleModel {
        responses: RefCell::new(responses.into_iter().map(str::to_string).collect()),
        images: Rc::clone(&images),
        prompts: Rc::clone(&prompts),
    };
    let runtime = Runtime::new(AgentRegistry::new(vec![Box::new(LocalAgent::new(
        definition(),
        Box::new(model),
    ))]))
    .with_tools(ToolRegistry::new(tools));
    (runtime, images, prompts)
}

#[test]
fn runtime_forwards_capture_as_image_without_embedding_bytes_in_tool_results() {
    const FIRST: &[u8] = b"FIRST_IMAGE_MARKER";
    let (runtime, images, prompts) = image_lifecycle_runtime(
        [
            r#"{"type":"tool_call","tool":"capture_first","args":{}}"#,
            r#"{"type":"final","content":"seen"}"#,
        ],
        vec![Box::new(SyntheticTool {
            name: "capture_first",
            output: SyntheticToolOutput::Image(FIRST),
        })],
    );

    assert_eq!(ask(&runtime, "inspect").unwrap(), "seen");
    let images = images.borrow();
    assert!(images[0].is_empty());
    assert_eq!(images[1][0].bytes, FIRST);
    let continuation = &prompts.borrow()[1];
    assert!(!continuation.contains("FIRST_IMAGE_MARKER"));
    assert!(!continuation.contains("data:image"));
}

#[test]
fn runtime_keeps_latest_image_across_text_tools_and_replaces_it_after_recapture() {
    const FIRST: &[u8] = b"FIRST_IMAGE_MARKER";
    const SECOND: &[u8] = b"SECOND_IMAGE_MARKER";
    let (runtime, images, _) = image_lifecycle_runtime(
        [
            r#"{"type":"tool_call","tool":"capture_first","args":{}}"#,
            r#"{"type":"tool_call","tool":"text","args":{}}"#,
            r#"{"type":"tool_call","tool":"capture_second","args":{}}"#,
            r#"{"type":"final","content":"done"}"#,
        ],
        vec![
            Box::new(SyntheticTool {
                name: "capture_first",
                output: SyntheticToolOutput::Image(FIRST),
            }),
            Box::new(SyntheticTool {
                name: "text",
                output: SyntheticToolOutput::Text,
            }),
            Box::new(SyntheticTool {
                name: "capture_second",
                output: SyntheticToolOutput::Image(SECOND),
            }),
        ],
    );

    assert_eq!(ask(&runtime, "inspect twice").unwrap(), "done");
    let markers = images
        .borrow()
        .iter()
        .map(|batch| batch.first().map(|image| image.bytes.clone()))
        .collect::<Vec<_>>();
    assert_eq!(
        markers,
        vec![
            None,
            Some(FIRST.to_vec()),
            Some(FIRST.to_vec()),
            Some(SECOND.to_vec())
        ]
    );
}

#[test]
fn runtime_keeps_last_successful_image_when_a_later_capture_fails() {
    const FIRST: &[u8] = b"FIRST_IMAGE_MARKER";
    let (runtime, images, _) = image_lifecycle_runtime(
        [
            r#"{"type":"tool_call","tool":"capture_first","args":{}}"#,
            r#"{"type":"tool_call","tool":"capture_fail","args":{}}"#,
            r#"{"type":"final","content":"recovered"}"#,
        ],
        vec![
            Box::new(SyntheticTool {
                name: "capture_first",
                output: SyntheticToolOutput::Image(FIRST),
            }),
            Box::new(SyntheticTool {
                name: "capture_fail",
                output: SyntheticToolOutput::Failure,
            }),
        ],
    );

    assert_eq!(ask(&runtime, "inspect safely").unwrap(), "recovered");
    let images = images.borrow();
    assert_eq!(images[1][0].bytes, FIRST);
    assert_eq!(images[2][0].bytes, FIRST);
}

#[test]
fn runtime_turns_provider_image_rejection_into_a_recoverable_tool_failure() {
    const FIRST: &[u8] = b"FIRST_IMAGE_MARKER";
    const OVERSIZED: &[u8] = b"OVERSIZED_IMAGE_MARKER";
    let (runtime, images, prompts) = image_lifecycle_runtime(
        [
            r#"{"type":"tool_call","tool":"capture_first","args":{}}"#,
            r#"{"type":"tool_call","tool":"capture_oversized","args":{}}"#,
            r#"{"type":"final","content":"recovered"}"#,
        ],
        vec![
            Box::new(SyntheticTool {
                name: "capture_first",
                output: SyntheticToolOutput::Image(FIRST),
            }),
            Box::new(SyntheticTool {
                name: "capture_oversized",
                output: SyntheticToolOutput::OversizedImage(OVERSIZED),
            }),
        ],
    );
    let events = Rc::new(RefCell::new(Vec::new()));

    let answer = runtime
        .ask_stream_with_history_and_authorization(
            "inspect safely",
            &[],
            PermissionMode::FullAccess,
            &AllowAllAuthorizer,
            &mut |_| {},
            &mut |event| events.borrow_mut().push(event.clone()),
        )
        .unwrap();

    assert_eq!(answer, "recovered");
    let images = images.borrow();
    assert_eq!(images[1][0].bytes, FIRST);
    assert_eq!(images[2][0].bytes, FIRST);
    let continuation = &prompts.borrow()[2];
    assert!(continuation.contains("synthetic provider rejects"));
    assert!(!continuation.contains("OVERSIZED_IMAGE_MARKER"));
    assert!(events.borrow().iter().any(|event| matches!(
        event,
        ToolEvent::Finished {
            tool,
            success: false,
            ..
        } if tool == "capture_oversized"
    )));
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
    assert!(matches!(events[0], ToolEvent::Status { .. }));
    assert!(matches!(events[1], ToolEvent::Usage { usage: None }));
    assert!(matches!(events[2], ToolEvent::Requested { .. }));
    assert!(matches!(events[3], ToolEvent::Started { .. }));
    assert!(matches!(
        events[4],
        ToolEvent::Finished { success: false, .. }
    ));
}

#[test]
fn runtime_emits_thinking_request_start_finish_and_preparing_statuses() {
    let workspace = tempfile::tempdir().unwrap();
    let model = ToolProtocolStreamingModel::new([
        vec![r#"{"type":"tool_call","tool":"pwd","args":{}}"#],
        vec![r#"{"type":"final","content":"done"}"#],
    ]);
    let runtime = Runtime::new(AgentRegistry::new(vec![Box::new(LocalAgent::new(
        definition(),
        Box::new(model),
    ))]))
    .with_tools(ToolRegistry::new(vec![Box::new(PwdTool::new(
        workspace.path(),
    ))]));
    let mut events = Vec::new();

    runtime
        .ask_stream_with_history_and_authorization(
            "locate",
            &[],
            PermissionMode::FullAccess,
            &AllowAllAuthorizer,
            &mut |_| {},
            &mut |event| events.push(event.clone()),
        )
        .unwrap();

    assert!(matches!(
        events.as_slice(),
        [
            ToolEvent::Status {
                status: RuntimeStatus::Thinking
            },
            ToolEvent::Usage { usage: None },
            ToolEvent::Requested {
                sequence: 0,
                tool,
                ..
            },
            ToolEvent::Started {
                sequence: 0,
                tool: started
            },
            ToolEvent::Finished {
                sequence: 0,
                tool: finished,
                success: true,
                ..
            },
            ToolEvent::Status {
                status: RuntimeStatus::PreparingAnswer
            },
            ToolEvent::Usage { usage: None }
        ] if tool == "pwd" && started == "pwd" && finished == "pwd"
    ));
}

#[test]
fn repeated_same_tool_calls_have_distinct_monotonic_sequences() {
    let workspace = tempfile::tempdir().unwrap();
    let model = ToolProtocolStreamingModel::new([
        vec![r#"{"type":"tool_call","tool":"pwd","args":{}}"#],
        vec![r#"{"type":"tool_call","tool":"pwd","args":{}}"#],
        vec![r#"{"type":"final","content":"done"}"#],
    ]);
    let runtime = Runtime::new(AgentRegistry::new(vec![Box::new(LocalAgent::new(
        definition(),
        Box::new(model),
    ))]))
    .with_tools(ToolRegistry::new(vec![Box::new(PwdTool::new(
        workspace.path(),
    ))]));
    let mut events = Vec::new();

    runtime
        .ask_stream_with_history_and_authorization(
            "locate twice",
            &[],
            PermissionMode::FullAccess,
            &AllowAllAuthorizer,
            &mut |_| {},
            &mut |event| events.push(event.clone()),
        )
        .unwrap();

    let requested = events
        .iter()
        .filter_map(|event| match event {
            ToolEvent::Requested { sequence, tool, .. } => Some((*sequence, tool.as_str())),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(requested, vec![(0, "pwd"), (1, "pwd")]);
}

#[test]
fn failed_tool_emits_sanitized_bounded_details() {
    let model = ToolProtocolStreamingModel::new([
        vec![r#"{"type":"tool_call","tool":"failing","args":{"apiKey":"private"}}"#],
        vec![r#"{"type":"final","content":"handled"}"#],
    ]);
    let runtime = Runtime::new(AgentRegistry::new(vec![Box::new(LocalAgent::new(
        definition(),
        Box::new(model),
    ))]))
    .with_tools(ToolRegistry::new(vec![Box::new(FailingTool)]));
    let mut events = Vec::new();

    runtime
        .ask_stream_with_history_and_authorization(
            "fail safely",
            &[],
            PermissionMode::FullAccess,
            &AllowAllAuthorizer,
            &mut |_| {},
            &mut |event| events.push(event.clone()),
        )
        .unwrap();

    assert!(events.iter().any(|event| matches!(
        event,
        ToolEvent::Requested { arguments, .. }
            if arguments.contains("[redacted]") && !arguments.contains("private")
    )));
    assert!(events.iter().any(|event| matches!(
        event,
        ToolEvent::Finished {
            success: false,
            details: Some(details),
            ..
        } if details == "simulated tool failure" && details.chars().count() <= 8_192
    )));
}

#[test]
fn screen_capture_event_contains_no_attachment_bytes() {
    const PIXELS: &[u8] = b"PRIVATE_SCREEN_PIXELS";
    let (runtime, _, _) = image_lifecycle_runtime(
        [
            r#"{"type":"tool_call","tool":"capture_screen","args":{}}"#,
            r#"{"type":"final","content":"seen"}"#,
        ],
        vec![Box::new(SyntheticTool {
            name: "capture_screen",
            output: SyntheticToolOutput::Image(PIXELS),
        })],
    );
    let mut events = Vec::new();

    runtime
        .ask_stream_with_history_and_authorization(
            "look",
            &[],
            PermissionMode::FullAccess,
            &AllowAllAuthorizer,
            &mut |_| {},
            &mut |event| events.push(event.clone()),
        )
        .unwrap();

    let serialized = serde_json::to_string(&events).unwrap();
    assert!(!serialized.contains("PRIVATE_SCREEN_PIXELS"));
    assert!(events.iter().any(|event| matches!(
        event,
        ToolEvent::Finished {
            tool,
            success: true,
            details: None,
            ..
        } if tool == "capture_screen"
    )));
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
            definition_with_protocol_repair_limit(1),
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
fn malformed_streamed_final_is_reset_before_repaired_answer() {
    let visible = Rc::new(RefCell::new(String::new()));
    let resets = Cell::new(0);
    let model = ToolProtocolStreamingModel::new([
        vec![r#"{"type":"final","content":"Discard me""#],
        vec![r#"{"type":"final","content":"Recovered"}"#],
    ]);
    let runtime = Runtime::new(AgentRegistry::new(vec![Box::new(LocalAgent::new(
        definition(),
        Box::new(model),
    ))]))
    .with_tools(ToolRegistry::new(vec![Box::new(PwdTool::new("."))]));
    let answer = runtime
        .ask_stream_with_history_and_authorization(
            "request",
            &[],
            PermissionMode::FullAccess,
            &AllowAllAuthorizer,
            &mut |delta| visible.borrow_mut().push_str(delta),
            &mut |event| {
                if serde_json::to_value(event).unwrap()["event"] == "responseReset" {
                    visible.borrow_mut().clear();
                    resets.set(resets.get() + 1);
                }
            },
        )
        .unwrap();
    assert_eq!(visible.borrow().as_str(), answer);
    assert_eq!(answer, "Recovered");
    assert_eq!(resets.get(), 1);
}

#[test]
fn runtime_keeps_repairing_protocol_errors_with_feedback_until_valid() {
    let requests = Rc::new(RefCell::new(Vec::new()));
    let model = HistoryCapturingStreamingModel::new(
        [
            vec!["plain response"],
            vec![r#"{"type":"final"}"#],
            vec![r#"{"content":"wrong field order","type":"final"}"#],
            vec![r#"{"type":"final","content":"Recovered answer"}"#],
        ],
        Rc::clone(&requests),
    );
    let runtime = Runtime::new(AgentRegistry::new(vec![Box::new(LocalAgent::new(
        definition(),
        Box::new(model),
    ))]))
    .with_tools(ToolRegistry::new(vec![Box::new(PwdTool::new("."))]));
    let mut deltas = Vec::new();
    let answer = runtime
        .ask_stream("original request", &mut |delta| {
            deltas.push(delta.to_owned())
        })
        .unwrap();
    assert_eq!(answer, "Recovered answer");
    assert_eq!(deltas, ["Recovered answer"]);
    let requests = requests.borrow();
    assert_eq!(requests.len(), 4);
    for (index, previous) in [
        "plain response",
        r#"{"type":"final"}"#,
        r#"{"content":"wrong field order","type":"final"}"#,
    ]
    .into_iter()
    .enumerate()
    {
        let repair: serde_json::Value =
            serde_json::from_str(&requests[index + 1].user_prompt).unwrap();
        assert_eq!(repair["type"], "protocol_repair");
        assert_eq!(repair["pending_request"], "original request");
        assert_eq!(repair["invalid_response"], previous);
        assert_eq!(repair["attempt"], index + 1);
        assert_eq!(
            repair["validation_error"],
            AgentOutput::parse_tool_protocol(previous).unwrap_err()
        );
    }
}

#[test]
fn repeated_protocol_repairs_preserve_tool_results_without_reexecuting_tools() {
    let requests = Rc::new(RefCell::new(Vec::new()));
    let executions = Rc::new(Cell::new(0));
    let model = HistoryCapturingStreamingModel::new(
        [
            vec![r#"{"type":"tool_call","tool":"pwd","args":{}}"#],
            vec!["bad response"],
            vec!["still bad"],
            vec![r#"{"type":"final","content":"Used saved tool result"}"#],
        ],
        Rc::clone(&requests),
    );
    let runtime = Runtime::new(AgentRegistry::new(vec![Box::new(LocalAgent::new(
        definition(),
        Box::new(model),
    ))]))
    .with_tools(ToolRegistry::new(vec![Box::new(CountingTool {
        executions: Rc::clone(&executions),
    })]));
    assert_eq!(
        ask(&runtime, "inspect folder").unwrap(),
        "Used saved tool result"
    );
    assert_eq!(executions.get(), 1);
    let requests = requests.borrow();
    for index in [2, 3] {
        let repair: serde_json::Value = serde_json::from_str(&requests[index].user_prompt).unwrap();
        assert_eq!(repair["pending_request"], requests[1].user_prompt);
        let continuation: serde_json::Value =
            serde_json::from_str(repair["pending_request"].as_str().unwrap()).unwrap();
        assert_eq!(continuation["tool_results"][0]["output"], "counted");
    }
}

#[test]
fn protocol_repairs_still_stop_when_context_preparation_fails() {
    use atlas_agentic::runtime::context::{
        ContextPreparationRequest, PreparedContext, RuntimeContext,
    };
    struct BudgetContext {
        calls: usize,
    }
    impl RuntimeContext for BudgetContext {
        fn prepare(
            &mut self,
            _: &dyn atlas_agentic::agents::Agent,
            _: ContextPreparationRequest<'_>,
            _: &mut dyn FnMut(&ToolEvent),
        ) -> Result<PreparedContext, String> {
            self.calls += 1;
            if self.calls == 3 {
                return Err("context budget exceeded".into());
            }
            Ok(PreparedContext {
                memory_blocks: vec![],
                history: vec![],
                activity_context: String::new(),
                max_output_tokens: None,
            })
        }
    }
    let runtime = Runtime::new(AgentRegistry::new(vec![Box::new(LocalAgent::new(
        definition(),
        Box::new(InvalidProtocolModel {
            response: "invalid response",
        }),
    ))]))
    .with_tools(ToolRegistry::new(vec![Box::new(PwdTool::new("."))]));
    let mut context = BudgetContext { calls: 0 };
    let result = runtime.ask_stream_with_managed_context(
        "original request",
        &mut context,
        PermissionMode::FullAccess,
        atlas_agentic::RuntimeAskCallbacks {
            authorizer: &AllowAllAuthorizer,
            on_delta: &mut |_| {},
            on_tool_event: &mut |_| {},
        },
    );
    assert_eq!(result.unwrap_err(), "context budget exceeded");
    assert_eq!(context.calls, 3);
}

#[test]
fn explicit_repair_limit_resets_after_a_valid_tool_call() {
    let executions = Rc::new(Cell::new(0));
    let model = ToolProtocolStreamingModel::new([
        vec!["invalid"],
        vec![r#"{"type":"tool_call","tool":"pwd","args":{}}"#],
        vec!["invalid again"],
        vec![r#"{"type":"final","content":"Recovered"}"#],
    ]);
    let runtime = Runtime::new(AgentRegistry::new(vec![Box::new(LocalAgent::new(
        definition_with_protocol_repair_limit(1),
        Box::new(model),
    ))]))
    .with_tools(ToolRegistry::new(vec![Box::new(CountingTool {
        executions: Rc::clone(&executions),
    })]));
    assert_eq!(ask(&runtime, "original request").unwrap(), "Recovered");
    assert_eq!(executions.get(), 1);
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
fn runtime_respects_an_explicit_protocol_repair_limit() {
    let workspace = tempfile::tempdir().unwrap();
    let model = ToolProtocolStreamingModel::new([vec![""], vec![""]]);
    let runtime = Runtime::new(AgentRegistry::new(vec![Box::new(LocalAgent::new(
        definition_with_protocol_repair_limit(1),
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
        definition_with_tool_call_limit(8),
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
        definition_with_tool_call_limit(8),
        Box::new(RepeatingMissingToolModel),
    ))]))
    .with_tools(ToolRegistry::new(vec![Box::new(PwdTool::new("."))]));

    let error = ask(&runtime, "keep trying a missing tool").unwrap_err();

    assert_eq!(error, "agent exceeded the maximum of 8 tool calls");
}

#[test]
fn runtime_allows_more_than_eight_tool_calls_when_no_limit_is_configured() {
    let executions = Rc::new(Cell::new(0));
    let mut responses = vec![r#"{"type":"tool_call","tool":"pwd","args":{}}"#; 9];
    responses.push(r#"{"type":"final","content":"Finished the long task."}"#);
    let runtime = Runtime::new(AgentRegistry::new(vec![Box::new(LocalAgent::new(
        definition(),
        Box::new(QueueModel::new(responses)),
    ))]))
    .with_tools(ToolRegistry::new(vec![Box::new(CountingTool {
        executions: Rc::clone(&executions),
    })]));

    let result = ask(&runtime, "complete a long tool sequence").unwrap();

    assert_eq!(result, "Finished the long task.");
    assert_eq!(executions.get(), 9);
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
