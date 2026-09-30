use std::{
    fs,
    path::{Component, Path, PathBuf},
};

use serde::Deserialize;

use super::{Tool, ToolDefinition, ToolResult};

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

    fn execute(&self, args: &serde_json::Value) -> Result<ToolResult, String> {
        let args: CreateFileArgs = serde_json::from_value(args.clone())
            .map_err(|error| format!("invalid create_file args: {error}"))?;
        validate_relative_path(&args.path)?;

        let path = self.workspace.join(&args.path);
        if path.exists() {
            return Err(format!("create_file target already exists: {}", args.path));
        }

        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .map_err(|error| format!("failed to create parent directories: {error}"))?;
        }

        fs::write(&path, args.content)
            .map_err(|error| format!("failed to create file: {error}"))?;

        Ok(ToolResult {
            message: format!("Created file: {}", args.path),
        })
    }
}

#[derive(Deserialize)]
struct CreateFileArgs {
    path: String,
    content: String,
}

fn validate_relative_path(path: &str) -> Result<(), String> {
    let path = Path::new(path);
    if path.as_os_str().is_empty() {
        return Err("create_file path is required".to_string());
    }
    if path.is_absolute() {
        return Err("create_file path must be relative".to_string());
    }
    if path
        .components()
        .any(|component| matches!(component, Component::ParentDir))
    {
        return Err("create_file path cannot contain parent directory segments".to_string());
    }
    Ok(())
}
