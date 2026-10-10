use chrono::Local;

use super::{Tool, ToolDefinition, ToolResult};

pub struct CurrentTimeTool;

impl CurrentTimeTool {
    pub fn new() -> Self {
        Self
    }
}

impl Default for CurrentTimeTool {
    fn default() -> Self {
        Self::new()
    }
}

impl Tool for CurrentTimeTool {
    fn name(&self) -> &str {
        "current_time"
    }

    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: self.name().to_string(),
            description: "Show the current date and time from the user's machine, including its local UTC offset."
                .to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {},
                "additionalProperties": false
            }),
        }
    }

    fn execute(&self, _args: &serde_json::Value) -> Result<ToolResult, String> {
        Ok(ToolResult {
            message: format!("Local date and time: {}", Local::now().to_rfc3339()),
        })
    }
}
