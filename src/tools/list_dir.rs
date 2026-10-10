use std::path::{Path, PathBuf};

use serde::Deserialize;

use super::{
    workspace_path::{open_workspace, validate_relative_path},
    Tool, ToolDefinition, ToolResult,
};

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
        let relative_path = validate_relative_path("ls", "path", &relative_path)?;
        let workspace = open_workspace(self.name(), &self.workspace)?;
        if !workspace
            .metadata(&relative_path)
            .map_err(|error| format!("failed to inspect ls target: {error}"))?
            .is_dir()
        {
            return Err(format!(
                "ls target is not a directory: {}",
                relative_path.display()
            ));
        }
        let mut entries = workspace
            .read_dir(&relative_path)
            .map_err(|error| format!("failed to list directory: {error}"))?
            .map(|entry| {
                entry
                    .map_err(|error| format!("failed to read directory entry: {error}"))
                    .map(|entry| {
                        if relative_path == Path::new(".") {
                            PathBuf::from(entry.file_name()).display().to_string()
                        } else {
                            relative_path.join(entry.file_name()).display().to_string()
                        }
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
