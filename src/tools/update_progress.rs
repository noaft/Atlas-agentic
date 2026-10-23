use serde::Deserialize;

use super::{Tool, ToolDefinition, ToolResult};

const MESSAGE_LIMIT: usize = 1_000;

#[derive(Default)]
pub struct UpdateProgressTool;

impl UpdateProgressTool {
    pub fn new() -> Self {
        Self
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Args {
    message: String,
}

impl Tool for UpdateProgressTool {
    fn name(&self) -> &str {
        "update_progress"
    }

    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: self.name().into(),
            description: "Show a brief user-facing update about what you are doing or will do next, then continue the task. Use before multi-step work and when the next step changes. Each update replaces the previous update; the final answer replaces the update. Describe actions or verified findings, not private reasoning or secrets.".into(),
            input_schema: serde_json::json!({
                "type": "object", "properties": {
                    "message": {"type": "string", "minLength": 1, "maxLength": MESSAGE_LIMIT}
                }, "required": ["message"], "additionalProperties": false
            }),
        }
    }

    fn execute(&self, args: &serde_json::Value) -> Result<ToolResult, String> {
        let args: Args = serde_json::from_value(args.clone())
            .map_err(|error| format!("invalid progress update arguments: {error}"))?;
        let message = args.message.trim();
        if message.is_empty() || message.chars().count() > MESSAGE_LIMIT {
            return Err(format!(
                "progress message must contain 1 to {MESSAGE_LIMIT} characters"
            ));
        }
        Ok(ToolResult::with_progress(message))
    }
}
