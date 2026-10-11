use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::permissions::ToolRisk;

use super::{
    workspace_path::{open_workspace, validate_relative_path},
    Tool, ToolDefinition, ToolResult,
};

pub struct CreateDirectoryTool {
    workspace: PathBuf,
}

impl CreateDirectoryTool {
    pub fn new(workspace: impl AsRef<Path>) -> Self {
        Self {
            workspace: workspace.as_ref().to_path_buf(),
        }
    }
}

impl Tool for CreateDirectoryTool {
    fn name(&self) -> &str {
        "create_directory"
    }

    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: self.name().to_string(),
            description: "Create a directory and missing parent directories inside the workspace."
                .to_string(),
            input_schema: serde_json::json!({
                "type":"object",
                "required":["path"],
                "properties":{"path":{"type":"string","description":"Relative directory path inside the workspace."}}
            }),
        }
    }

    fn risk(&self, args: &serde_json::Value) -> Result<ToolRisk, String> {
        parse(args)?;
        Ok(ToolRisk::ModifyWorkspace)
    }

    fn execute(&self, args: &serde_json::Value) -> Result<ToolResult, String> {
        let path = parse(args)?;
        let workspace = open_workspace(self.name(), &self.workspace)?;
        workspace
            .create_dir_all(&path)
            .map_err(|error| format!("failed to create directory {}: {error}", path.display()))?;
        if !workspace
            .metadata(&path)
            .map_err(|error| format!("failed to inspect directory {}: {error}", path.display()))?
            .is_dir()
        {
            return Err(format!(
                "create_directory target is not a directory: {}",
                path.display()
            ));
        }
        Ok(ToolResult::text(format!(
            "Created directory: {}",
            path.display()
        )))
    }
}

#[derive(Deserialize)]
struct Args {
    path: String,
}

fn parse(args: &serde_json::Value) -> Result<PathBuf, String> {
    let args: Args = serde_json::from_value(args.clone())
        .map_err(|error| format!("invalid create_directory args: {error}"))?;
    validate_relative_path("create_directory", "path", &args.path)
}
