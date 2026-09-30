use std::{cell::RefCell, collections::VecDeque, fs};

use atlas_agentic::{
    agent_output::AgentOutput,
    agents::{AgentRegistry, LocalAgent, LocalAgentDefinition},
    models::{ModelProvider, ModelRequest},
    tools::{CreateFileTool, ListDirTool, PwdTool, ToolRegistry},
    Runtime,
};

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

        Ok(format!("{} tools available", tool_names.join(",")))
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
"#,
    )
    .unwrap()
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
    let model = QueueModel::new([r#"
{
  "type": "tool_call",
  "tool": "create_file",
  "args": {
    "path": "notes/hello.txt",
    "content": "Hello from Atlas"
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

    let result = runtime.ask("create a file").unwrap();

    assert_eq!(result, "Created file: notes/hello.txt");
    assert_eq!(
        fs::read_to_string(workspace.path().join("notes/hello.txt")).unwrap(),
        "Hello from Atlas"
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

    let error = runtime.ask("create unsafe file").unwrap_err();

    assert_eq!(
        error,
        "create_file path cannot contain parent directory segments"
    );
}

#[test]
fn runtime_passes_registered_tools_to_model_request() {
    let workspace = tempfile::tempdir().unwrap();
    let runtime = Runtime::new(AgentRegistry::new(vec![Box::new(LocalAgent::new(
        definition(),
        Box::new(ToolAwareModel),
    ))]))
    .with_tools(ToolRegistry::new(vec![
        Box::new(CreateFileTool::new(workspace.path())),
        Box::new(PwdTool::new(workspace.path())),
        Box::new(ListDirTool::new(workspace.path())),
    ]));

    let result = runtime.ask("what tools exist?").unwrap();

    assert_eq!(result, "create_file,ls,pwd tools available");
}

#[test]
fn runtime_executes_pwd_tool_call() {
    let workspace = tempfile::tempdir().unwrap();
    let model = QueueModel::new([r#"
{
  "type": "tool_call",
  "tool": "pwd",
  "args": {}
}
"#]);
    let runtime = Runtime::new(AgentRegistry::new(vec![Box::new(LocalAgent::new(
        definition(),
        Box::new(model),
    ))]))
    .with_tools(ToolRegistry::new(vec![Box::new(PwdTool::new(
        workspace.path(),
    ))]));

    let result = runtime.ask("where am I?").unwrap();

    assert_eq!(result, format!("Workspace: {}", workspace.path().display()));
}

#[test]
fn runtime_executes_ls_tool_call() {
    let workspace = tempfile::tempdir().unwrap();
    fs::create_dir(workspace.path().join("notes")).unwrap();
    fs::write(workspace.path().join("notes/a.txt"), "a").unwrap();
    fs::write(workspace.path().join("notes/b.txt"), "b").unwrap();
    let model = QueueModel::new([r#"
{
  "type": "tool_call",
  "tool": "ls",
  "args": {
    "path": "notes"
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

    let result = runtime.ask("list notes").unwrap();

    assert_eq!(result, "notes/a.txt\nnotes/b.txt");
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

    let error = runtime.ask("list outside").unwrap_err();

    assert_eq!(error, "ls path cannot contain parent directory segments");
}
