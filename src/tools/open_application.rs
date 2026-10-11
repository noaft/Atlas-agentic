use std::sync::Arc;

use serde::Deserialize;

use crate::permissions::ToolRisk;

use super::{
    platform::{application_command, system_launcher, Platform, ProcessLauncher},
    Tool, ToolDefinition, ToolResult,
};

pub struct OpenApplicationTool {
    platform: Platform,
    launcher: Arc<dyn ProcessLauncher>,
}

impl OpenApplicationTool {
    pub fn new() -> Self {
        Self::with_launcher(Platform::current(), system_launcher())
    }

    #[doc(hidden)]
    pub fn with_launcher(platform: Platform, launcher: Arc<dyn ProcessLauncher>) -> Self {
        Self { platform, launcher }
    }
}

impl Default for OpenApplicationTool {
    fn default() -> Self {
        Self::new()
    }
}

impl Tool for OpenApplicationTool {
    fn name(&self) -> &str {
        "open_application"
    }

    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: self.name().to_string(),
            description: "Open an application with optional arguments.".to_string(),
            input_schema: serde_json::json!({
                "type":"object",
                "required":["application"],
                "properties":{
                    "application":{"type":"string"},
                    "args":{"type":"array","items":{"type":"string"},"default":[]}
                }
            }),
        }
    }

    fn risk(&self, args: &serde_json::Value) -> Result<ToolRisk, String> {
        parse(args)?;
        Ok(ToolRisk::Launch)
    }

    fn execute(&self, args: &serde_json::Value) -> Result<ToolResult, String> {
        let args = parse(args)?;
        self.launcher.launch(&application_command(
            self.platform,
            &args.application,
            &args.args,
        ))?;
        Ok(ToolResult::text(format!(
            "Opened application: {}",
            args.application
        )))
    }
}

#[derive(Deserialize)]
struct Args {
    application: String,
    #[serde(default)]
    args: Vec<String>,
}

fn parse(args: &serde_json::Value) -> Result<Args, String> {
    let mut args: Args = serde_json::from_value(args.clone())
        .map_err(|error| format!("invalid open_application args: {error}"))?;
    args.application = args.application.trim().to_string();
    if args.application.is_empty() {
        return Err("open_application application is required".to_string());
    }
    Ok(args)
}
