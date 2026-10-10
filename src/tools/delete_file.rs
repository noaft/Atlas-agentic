use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::permissions::ToolRisk;

use super::{
    workspace_path::{open_workspace, require_regular_file, validate_relative_path},
    Tool, ToolDefinition, ToolResult,
};

pub struct DeleteFileTool {
    workspace: PathBuf,
}

impl DeleteFileTool {
    pub fn new(workspace: impl AsRef<Path>) -> Self {
        Self {
            workspace: workspace.as_ref().to_path_buf(),
        }
    }
}

impl Tool for DeleteFileTool {
    fn name(&self) -> &str {
        "delete_file"
    }

    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: self.name().to_string(),
            description: "Delete one regular file inside the workspace.".to_string(),
            input_schema: serde_json::json!({
                "type":"object",
                "required":["path"],
                "properties":{"path":{"type":"string"}}
            }),
        }
    }

    fn risk(&self, args: &serde_json::Value) -> Result<ToolRisk, String> {
        parse(args)?;
        Ok(ToolRisk::Destructive)
    }

    fn execute(&self, args: &serde_json::Value) -> Result<ToolResult, String> {
        let path = parse(args)?;
        let workspace = open_workspace(self.name(), &self.workspace)?;
        require_regular_file(self.name(), "target", &workspace, &path)?;
        workspace
            .remove_file(&path)
            .map_err(|error| format!("failed to delete file {}: {error}", path.display()))?;
        Ok(ToolResult {
            message: format!("Deleted file: {}", path.display()),
        })
    }
}

#[derive(Deserialize)]
struct Args {
    path: String,
}

fn parse(args: &serde_json::Value) -> Result<PathBuf, String> {
    let args: Args = serde_json::from_value(args.clone())
        .map_err(|error| format!("invalid delete_file args: {error}"))?;
    validate_relative_path("delete_file", "path", &args.path)
}
