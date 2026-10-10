use std::{
    io,
    path::{Path, PathBuf},
};

use cap_std::fs::OpenOptions;
use serde::Deserialize;

use crate::permissions::ToolRisk;

use super::{
    workspace_path::{open_workspace, require_regular_file, validate_relative_path},
    Tool, ToolDefinition, ToolResult,
};

pub struct CopyFileTool {
    workspace: PathBuf,
}

impl CopyFileTool {
    pub fn new(workspace: impl AsRef<Path>) -> Self {
        Self {
            workspace: workspace.as_ref().to_path_buf(),
        }
    }
}

impl Tool for CopyFileTool {
    fn name(&self) -> &str {
        "copy_file"
    }

    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: self.name().to_string(),
            description: "Copy one file to another path inside the workspace.".to_string(),
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
        let workspace = open_workspace(self.name(), &self.workspace)?;
        copy(&workspace, &args.source, &args.destination, args.overwrite)?;
        Ok(ToolResult {
            message: format!(
                "Copied file: {} -> {}",
                args.source.display(),
                args.destination.display()
            ),
        })
    }
}

pub(super) fn copy(
    workspace: &cap_std::fs::Dir,
    source: &Path,
    destination: &Path,
    overwrite: bool,
) -> Result<(), String> {
    if source == destination {
        return Err("copy_file source and destination must be different".to_string());
    }
    require_regular_file("copy_file", "source", workspace, source)?;
    reject_same_file("copy_file", workspace, source, destination)?;
    let mut source_file = workspace
        .open(source)
        .map_err(|error| format!("failed to open copy_file source: {error}"))?;
    let mut options = OpenOptions::new();
    options.write(true);
    if overwrite {
        options.create(true).truncate(true);
    } else {
        options.create_new(true);
    }
    let mut destination_file = workspace
        .open_with(destination, &options)
        .map_err(|error| {
            if !overwrite && error.kind() == std::io::ErrorKind::AlreadyExists {
                format!(
                    "copy_file destination already exists: {}",
                    destination.display()
                )
            } else {
                format!("failed to open copy_file destination: {error}")
            }
        })?;
    if let Err(error) = io::copy(&mut source_file, &mut destination_file) {
        if !overwrite {
            let _ = workspace.remove_file(destination);
        }
        return Err(format!("failed to copy file: {error}"));
    }
    Ok(())
}

pub(super) fn reject_same_file(
    tool: &str,
    workspace: &cap_std::fs::Dir,
    source: &Path,
    destination: &Path,
) -> Result<(), String> {
    let destination_file = match workspace.open(destination) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(format!("failed to inspect {tool} destination: {error}")),
    };
    let source_file = workspace
        .open(source)
        .map_err(|error| format!("failed to inspect {tool} source: {error}"))?;
    let source_handle = same_file::Handle::from_file(source_file.into_std())
        .map_err(|error| format!("failed to inspect {tool} source identity: {error}"))?;
    let destination_handle = same_file::Handle::from_file(destination_file.into_std())
        .map_err(|error| format!("failed to inspect {tool} destination identity: {error}"))?;
    if source_handle == destination_handle {
        return Err(format!(
            "{tool} source and destination must be different files"
        ));
    }
    Ok(())
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
        .map_err(|error| format!("invalid copy_file args: {error}"))?;
    Ok(Args {
        source: validate_relative_path("copy_file", "source", &args.source)?,
        destination: validate_relative_path("copy_file", "destination", &args.destination)?,
        overwrite: args.overwrite,
    })
}
