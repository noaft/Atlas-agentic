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
            description: "Tell the user your next concrete action and why it matters, then continue working. Call this BEFORE starting multi-step work and again when the next meaningful step changes; do not wait until the final answer to communicate. Write 1-2 short sentences in the user's language. Example: I will inspect the project files first so I can follow the existing structure. Later: The changes are ready. I will run the tests next to verify the behavior. Describe planned actions or verified findings; do not claim planned work is complete, expose private reasoning or secrets, repeat generic status labels, or announce every tool call. Simple direct answers need no update. Each update replaces the previous update, and the final answer replaces it. This tool does not finish the task.".into(),
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
