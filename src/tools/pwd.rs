use std::path::{Path, PathBuf};

use super::{Tool, ToolDefinition, ToolResult};

pub struct PwdTool {
    workspace: PathBuf,
}

impl PwdTool {
    pub fn new(workspace: impl AsRef<Path>) -> Self {
        Self {
            workspace: workspace.as_ref().to_path_buf(),
        }
    }
}

impl Tool for PwdTool {
    fn name(&self) -> &str {
        "pwd"
    }

    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: self.name().to_string(),
            description: "Show the current Atlas workspace path.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {}
            }),
        }
    }

    fn execute(&self, _args: &serde_json::Value) -> Result<ToolResult, String> {
        Ok(ToolResult {
            message: format!("Workspace: {}", self.workspace.display()),
        })
    }
}
