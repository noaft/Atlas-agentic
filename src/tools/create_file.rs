use std::{
    io::Write,
    path::{Path, PathBuf},
};

use cap_std::fs::OpenOptions;
use serde::Deserialize;

use crate::permissions::ToolRisk;

use super::{
    workspace_path::{open_workspace, validate_relative_path},
    Tool, ToolDefinition, ToolResult,
};

pub struct CreateFileTool {
    workspace: PathBuf,
}

impl CreateFileTool {
    pub fn new(workspace: impl AsRef<Path>) -> Self {
        Self {
            workspace: workspace.as_ref().to_path_buf(),
        }
    }
}

impl Tool for CreateFileTool {
    fn name(&self) -> &str {
        "create_file"
    }

    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: self.name().to_string(),
            description:
                "Create a new file inside the workspace. Fails if the file already exists."
                    .to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "required": ["path", "content"],
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "Relative path inside the workspace."
                    },
                    "content": {
                        "type": "string",
                        "description": "File content to write."
                    }
                }
            }),
        }
    }

    fn risk(&self, args: &serde_json::Value) -> Result<ToolRisk, String> {
        let args: CreateFileArgs = serde_json::from_value(args.clone())
            .map_err(|error| format!("invalid create_file args: {error}"))?;
        validate_relative_path("create_file", "path", &args.path)?;
        Ok(ToolRisk::ModifyWorkspace)
    }

    fn execute(&self, args: &serde_json::Value) -> Result<ToolResult, String> {
        let args: CreateFileArgs = serde_json::from_value(args.clone())
            .map_err(|error| format!("invalid create_file args: {error}"))?;
        let path = validate_relative_path("create_file", "path", &args.path)?;
        let workspace = open_workspace(self.name(), &self.workspace)?;
        if let Some(parent) = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
        {
            workspace
                .create_dir_all(parent)
                .map_err(|error| format!("failed to create parent directories: {error}"))?;
        }
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        let mut file = workspace.open_with(&path, &options).map_err(|error| {
            if error.kind() == std::io::ErrorKind::AlreadyExists {
                format!("create_file target already exists: {}", args.path)
            } else {
                format!("failed to create file: {error}")
            }
        })?;
        file.write_all(args.content.as_bytes())
            .map_err(|error| format!("failed to create file: {error}"))?;

        Ok(ToolResult::text(format!("Created file: {}", args.path)))
    }
}

#[derive(Deserialize)]
struct CreateFileArgs {
    path: String,
    content: String,
}
