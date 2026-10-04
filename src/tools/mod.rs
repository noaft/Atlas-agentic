mod create_file;
#[cfg(unix)]
mod exec_command;
mod list_dir;
mod pwd;
mod read_file;

pub use create_file::CreateFileTool;
#[cfg(unix)]
pub use exec_command::ExecCommandTool;
pub use list_dir::ListDirTool;
pub use pwd::PwdTool;
pub use read_file::ReadFileTool;

use std::collections::HashMap;

use crate::agent_output::AgentOutput;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolDefinition {
    pub name: String,
    pub description: String,
    pub input_schema: serde_json::Value,
}

pub struct ToolResult {
    pub message: String,
}

pub trait Tool {
    fn name(&self) -> &str;
    fn definition(&self) -> ToolDefinition;
    fn execute(&self, args: &serde_json::Value) -> Result<ToolResult, String>;
}

pub struct ToolRegistry {
    tools: HashMap<String, Box<dyn Tool>>,
}

impl ToolRegistry {
    pub fn empty() -> Self {
        Self {
            tools: HashMap::new(),
        }
    }

    pub fn new(tools: Vec<Box<dyn Tool>>) -> Self {
        let mut registry = Self::empty();
        for tool in tools {
            registry.register(tool);
        }
        registry
    }

    pub fn register(&mut self, tool: Box<dyn Tool>) {
        self.tools.insert(tool.name().to_string(), tool);
    }

    pub fn execute(&self, output: &AgentOutput) -> Result<ToolResult, String> {
        match output {
            AgentOutput::Final { content } => Ok(ToolResult {
                message: content.clone(),
            }),
            AgentOutput::ToolCall { tool, args } => self
                .tools
                .get(tool)
                .ok_or_else(|| format!("unknown tool: {tool}"))?
                .execute(args),
        }
    }

    pub fn definitions(&self) -> Vec<ToolDefinition> {
        self.tools.values().map(|tool| tool.definition()).collect()
    }
}
