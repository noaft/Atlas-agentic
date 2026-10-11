use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

use serde::Deserialize;

use crate::permissions::ToolRisk;

use super::{
    platform::{system_launcher, Platform, ProcessLauncher},
    workspace_path::{open_workspace, require_regular_file, validate_relative_path},
    Tool, ToolDefinition, ToolResult,
};

pub struct OpenFileTool {
    workspace: PathBuf,
    platform: Platform,
    launcher: Arc<dyn ProcessLauncher>,
}

impl OpenFileTool {
    pub fn new(workspace: impl AsRef<Path>) -> Self {
        Self::with_launcher(workspace, Platform::current(), system_launcher())
    }

    #[doc(hidden)]
    pub fn with_launcher(
        workspace: impl AsRef<Path>,
        platform: Platform,
        launcher: Arc<dyn ProcessLauncher>,
    ) -> Self {
        Self {
            workspace: workspace.as_ref().to_path_buf(),
            platform,
            launcher,
        }
    }
}

impl Tool for OpenFileTool {
    fn name(&self) -> &str {
        "open_file"
    }

    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: self.name().to_string(),
            description: "Open a workspace file with the operating system's default application."
                .to_string(),
            input_schema: serde_json::json!({
                "type":"object",
                "required":["path"],
                "properties":{"path":{"type":"string"}}
            }),
        }
    }

    fn risk(&self, args: &serde_json::Value) -> Result<ToolRisk, String> {
        parse(args)?;
        Ok(ToolRisk::Launch)
    }

    fn execute(&self, args: &serde_json::Value) -> Result<ToolResult, String> {
        let path = parse(args)?;
        let workspace = open_workspace(self.name(), &self.workspace)?;
        require_regular_file(self.name(), "target", &workspace, &path)?;
        let absolute = self.workspace.join(&path);
        let absolute = absolute
            .to_str()
            .ok_or_else(|| "open_file path must contain valid Unicode".to_string())?;
        self.launcher.launch_file(self.platform, absolute)?;
        Ok(ToolResult::text(format!("Opened file: {}", path.display())))
    }
}

#[derive(Deserialize)]
struct Args {
    path: String,
}

fn parse(args: &serde_json::Value) -> Result<PathBuf, String> {
    let args: Args = serde_json::from_value(args.clone())
        .map_err(|error| format!("invalid open_file args: {error}"))?;
    validate_relative_path("open_file", "path", &args.path)
}
