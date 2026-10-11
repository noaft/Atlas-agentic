use serde::Deserialize;

use crate::{
    media::{ImageData, ImageDetail},
    permissions::ToolRisk,
    tools::{Tool, ToolDefinition, ToolResult},
};

use super::controller::SharedComputerController;

pub struct CaptureScreenTool {
    controller: SharedComputerController,
}

impl CaptureScreenTool {
    pub fn new(controller: SharedComputerController) -> Self {
        Self { controller }
    }
}

impl Tool for CaptureScreenTool {
    fn name(&self) -> &str {
        "capture_screen"
    }

    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: self.name().to_string(),
            description: "Capture the primary display as an in-memory PNG for visual inspection."
                .to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {},
                "additionalProperties": false
            }),
        }
    }

    fn risk(&self, args: &serde_json::Value) -> Result<ToolRisk, String> {
        parse(args)?;
        Ok(ToolRisk::ComputerObserve)
    }

    fn summarize(&self, args: &serde_json::Value) -> String {
        if parse(args).is_err() {
            return super::super::summarize_arguments(args);
        }
        r#"{"scope":"primary_display"}"#.to_string()
    }

    fn execute(&self, args: &serde_json::Value) -> Result<ToolResult, String> {
        parse(args)?;
        let captured = self
            .controller
            .lock()
            .map_err(|_| "computer controller lock was poisoned".to_string())?
            .capture_primary_display()?;
        if captured.width == 0 || captured.height == 0 {
            return Err("capture_screen returned zero-sized dimensions".to_string());
        }
        if captured.png.is_empty() {
            return Err("capture_screen returned an empty PNG".to_string());
        }
        let message = format!(
            "Captured primary display as PNG ({}x{}).",
            captured.width, captured.height
        );
        Ok(ToolResult::with_image(
            message,
            ImageData {
                mime_type: "image/png".to_string(),
                bytes: captured.png,
                width: captured.width,
                height: captured.height,
                detail: ImageDetail::Original,
            },
        ))
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Args {}

fn parse(args: &serde_json::Value) -> Result<Args, String> {
    serde_json::from_value(args.clone())
        .map_err(|error| format!("invalid capture_screen args: {error}"))
}
