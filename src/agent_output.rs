use serde::Deserialize;

pub(crate) const TOOL_MESSAGE_LIMIT: usize = 1_000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AgentOutput {
    Final {
        content: String,
    },
    ToolCall {
        tool: String,
        message: Option<String>,
        args: serde_json::Value,
    },
}

impl AgentOutput {
    pub fn parse(input: &str) -> Result<Self, String> {
        let trimmed = input.trim();
        if trimmed.is_empty() {
            return Ok(Self::Final {
                content: String::new(),
            });
        }

        match serde_json::from_str::<RawAgentOutput>(trimmed) {
            Ok(raw) => raw.into_agent_output(),
            Err(_) => Ok(Self::Final {
                content: input.to_string(),
            }),
        }
    }

    pub fn parse_tool_protocol(input: &str) -> Result<Self, String> {
        let trimmed = input.trim();
        if trimmed.is_empty() {
            return Err("invalid agent tool protocol: response was empty".to_string());
        }

        let raw = serde_json::from_str::<RawAgentOutput>(trimmed)
            .map_err(|error| format!("invalid agent tool protocol: {error}"))?;
        if first_object_key(trimmed).as_deref() != Some("type") {
            return Err("invalid agent tool protocol: type must be the first field".to_string());
        }

        raw.into_agent_output()
            .map_err(|error| format!("invalid agent tool protocol: {error}"))
    }
}

fn first_object_key(input: &str) -> Option<String> {
    let object = input.trim_start().strip_prefix('{')?.trim_start();
    let mut deserializer = serde_json::Deserializer::from_str(object);
    String::deserialize(&mut deserializer).ok()
}

#[derive(Deserialize)]
#[serde(tag = "type")]
enum RawAgentOutput {
    #[serde(rename = "final")]
    Final { content: String },
    #[serde(rename = "tool_call")]
    ToolCall {
        tool: String,
        message: Option<String>,
        #[serde(default)]
        args: serde_json::Value,
    },
}

impl RawAgentOutput {
    fn into_agent_output(self) -> Result<AgentOutput, String> {
        match self {
            Self::Final { content } => Ok(AgentOutput::Final { content }),
            Self::ToolCall {
                tool,
                args,
                message,
            } => {
                if tool.trim().is_empty() {
                    Err("tool_call needs tool".to_string())
                } else {
                    let message = match message {
                        Some(message) => {
                            let message = message.trim();
                            if message.is_empty() || message.chars().count() > TOOL_MESSAGE_LIMIT {
                                return Err(format!("tool_call message must contain 1 to {TOOL_MESSAGE_LIMIT} characters"));
                            }
                            Some(message.to_string())
                        }
                        None => None,
                    };
                    Ok(AgentOutput::ToolCall {
                        tool,
                        args,
                        message,
                    })
                }
            }
        }
    }
}
