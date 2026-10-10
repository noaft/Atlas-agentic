use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::permissions::ToolRisk;

use super::{
    copy_file::{copy, reject_same_file},
    workspace_path::{open_workspace, require_regular_file, validate_relative_path},
    Tool, ToolDefinition, ToolResult,
};

pub struct MoveFileTool {
    workspace: PathBuf,
}

impl MoveFileTool {
    pub fn new(workspace: impl AsRef<Path>) -> Self {
        Self {
            workspace: workspace.as_ref().to_path_buf(),
        }
    }
}

impl Tool for MoveFileTool {
    fn name(&self) -> &str {
        "move_file"
    }

    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: self.name().to_string(),
            description: "Move one file to another path inside the workspace.".to_string(),
            input_schema: serde_json::json!({
                "type":"object",
                "required":["source","destination"],
                "properties":{
                    "source":{"type":"string"},
                    "destination":{"type":"string"},
                    "overwrite":{"type":"boolean","default":false}
                }
            }),
        }
    }

    fn risk(&self, args: &serde_json::Value) -> Result<ToolRisk, String> {
        let args = parse(args)?;
        Ok(if args.overwrite {
            ToolRisk::Destructive
        } else {
            ToolRisk::ModifyWorkspace
        })
    }

    fn execute(&self, args: &serde_json::Value) -> Result<ToolResult, String> {
        let args = parse(args)?;
        if args.source == args.destination {
            return Err("move_file source and destination must be different".to_string());
        }
        let workspace = open_workspace(self.name(), &self.workspace)?;
        require_regular_file(self.name(), "source", &workspace, &args.source)?;
        reject_same_file(self.name(), &workspace, &args.source, &args.destination)?;

        let moved_without_copy = if args.overwrite {
            match workspace.rename(&args.source, &workspace, &args.destination) {
                Ok(()) => true,
                Err(error) if rename_should_fall_back(error.kind(), cfg!(windows)) => false,
                Err(error) => return Err(format!("failed to rename move_file source: {error}")),
            }
        } else {
            match workspace.hard_link(&args.source, &workspace, &args.destination) {
                Ok(()) => {
                    workspace.remove_file(&args.source).map_err(|error| {
                        let _ = workspace.remove_file(&args.destination);
                        format!("failed to remove move_file source: {error}")
                    })?;
                    true
                }
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                    return Err(format!(
                        "move_file destination already exists: {}",
                        args.destination.display()
                    ));
                }
                // A hard link is only an optimization. Filesystems may reject it
                // even when an ordinary create-new copy is supported.
                Err(_) => false,
            }
        };

        if !moved_without_copy {
            copy(&workspace, &args.source, &args.destination, args.overwrite)
                .map_err(|error| error.replacen("copy_file", "move_file", 1))?;
            if let Err(error) = workspace.remove_file(&args.source) {
                if !args.overwrite {
                    let _ = workspace.remove_file(&args.destination);
                }
                return Err(format!("failed to remove move_file source: {error}"));
            }
        }
        Ok(ToolResult {
            message: format!(
                "Moved file: {} -> {}",
                args.source.display(),
                args.destination.display()
            ),
        })
    }
}

fn rename_should_fall_back(kind: std::io::ErrorKind, windows: bool) -> bool {
    kind == std::io::ErrorKind::CrossesDevices
        || (windows && kind == std::io::ErrorKind::AlreadyExists)
}

#[derive(Deserialize)]
struct RawArgs {
    source: String,
    destination: String,
    #[serde(default)]
    overwrite: bool,
}

struct Args {
    source: PathBuf,
    destination: PathBuf,
    overwrite: bool,
}

fn parse(args: &serde_json::Value) -> Result<Args, String> {
    let args: RawArgs = serde_json::from_value(args.clone())
        .map_err(|error| format!("invalid move_file args: {error}"))?;
    Ok(Args {
        source: validate_relative_path("move_file", "source", &args.source)?,
        destination: validate_relative_path("move_file", "destination", &args.destination)?,
        overwrite: args.overwrite,
    })
}

#[cfg(test)]
mod tests {
    use std::io::ErrorKind;

    use super::rename_should_fall_back;

    #[test]
    fn windows_existing_destination_falls_back_to_overwriting_copy() {
        assert!(rename_should_fall_back(ErrorKind::AlreadyExists, true));
        assert!(!rename_should_fall_back(ErrorKind::AlreadyExists, false));
        assert!(rename_should_fall_back(ErrorKind::CrossesDevices, false));
    }
}
