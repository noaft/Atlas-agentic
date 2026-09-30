use std::{
    fs,
    path::{Component, Path, PathBuf},
};

use serde::Deserialize;

use super::{Tool, ToolDefinition, ToolResult};

pub struct ListDirTool {
    workspace: PathBuf,
}

impl ListDirTool {
    pub fn new(workspace: impl AsRef<Path>) -> Self {
        Self {
            workspace: workspace.as_ref().to_path_buf(),
        }
    }
}

impl Tool for ListDirTool {
    fn name(&self) -> &str {
        "ls"
    }

    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: self.name().to_string(),
            description: "List files and directories inside the workspace.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "Relative directory path inside the workspace. Defaults to the workspace root."
                    }
                }
            }),
        }
    }

    fn execute(&self, args: &serde_json::Value) -> Result<ToolResult, String> {
        let args: ListDirArgs = serde_json::from_value(args.clone())
            .map_err(|error| format!("invalid ls args: {error}"))?;
        let relative_path = args.path.unwrap_or_else(|| ".".to_string());
        validate_relative_path(&relative_path)?;

        let directory = self.workspace.join(&relative_path);
        if !directory.is_dir() {
            return Err(format!("ls target is not a directory: {relative_path}"));
        }

        let mut entries = fs::read_dir(&directory)
            .map_err(|error| format!("failed to list directory: {error}"))?
            .map(|entry| {
                entry
                    .map_err(|error| format!("failed to read directory entry: {error}"))
                    .and_then(|entry| {
                        let path = entry.path();
                        path.strip_prefix(&self.workspace)
                            .map(|path| path.display().to_string())
                            .map_err(|error| format!("failed to format directory entry: {error}"))
                    })
            })
            .collect::<Result<Vec<_>, _>>()?;

        entries.sort();

        Ok(ToolResult {
            message: entries.join("\n"),
        })
    }
}

#[derive(Deserialize)]
struct ListDirArgs {
    path: Option<String>,
}

fn validate_relative_path(path: &str) -> Result<(), String> {
    let path = Path::new(path);
    if path.as_os_str().is_empty() {
        return Err("ls path is required".to_string());
    }
    if path.is_absolute() {
        return Err("ls path must be relative".to_string());
    }
    if path
        .components()
        .any(|component| matches!(component, Component::ParentDir))
    {
        return Err("ls path cannot contain parent directory segments".to_string());
    }
    Ok(())
}
