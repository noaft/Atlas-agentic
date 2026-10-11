use serde::Deserialize;

use crate::{
    permissions::ToolRisk,
    tools::{Tool, ToolDefinition, ToolResult},
};

use super::SharedComputerController;

pub struct GetUiStateTool {
    controller: SharedComputerController,
}

impl GetUiStateTool {
    pub fn new(controller: SharedComputerController) -> Self {
        Self { controller }
    }
}

impl Tool for GetUiStateTool {
    fn name(&self) -> &str {
        "get_ui_state"
    }

    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: self.name().to_string(),
            description:
                "Read pointer, primary-display, and active-window metadata from the desktop."
                    .to_string(),
            input_schema: serde_json::json!({"type": "object", "properties": {}}),
        }
    }

    fn risk(&self, args: &serde_json::Value) -> Result<ToolRisk, String> {
        parse(args)?;
        Ok(ToolRisk::ComputerObserve)
    }

    fn execute(&self, args: &serde_json::Value) -> Result<ToolResult, String> {
        parse(args)?;
        let state = self
            .controller
            .lock()
            .map_err(|_| "computer controller lock was poisoned".to_string())?
            .ui_state()?;
        let message = serde_json::to_string_pretty(&state)
            .map_err(|error| format!("failed to serialize UI state: {error}"))?;
        Ok(ToolResult::text(message))
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct EmptyArgs {}

fn parse(args: &serde_json::Value) -> Result<EmptyArgs, String> {
    serde_json::from_value(args.clone())
        .map_err(|error| format!("invalid get_ui_state args: {error}"))
}
