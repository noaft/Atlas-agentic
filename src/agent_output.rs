use serde::Deserialize;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AgentOutput {
    Final {
        content: String,
    },
    ToolCall {
        tool: String,
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

        serde_json::from_str::<RawAgentOutput>(trimmed)
            .map_err(|error| format!("invalid agent tool protocol: {error}"))?
            .into_agent_output()
            .map_err(|error| format!("invalid agent tool protocol: {error}"))
    }
}

#[derive(Deserialize)]
#[serde(tag = "type")]
enum RawAgentOutput {
    #[serde(rename = "final")]
    Final { content: String },
    #[serde(rename = "tool_call")]
    ToolCall {
        tool: String,
        #[serde(default)]
        args: serde_json::Value,
    },
}

impl RawAgentOutput {
    fn into_agent_output(self) -> Result<AgentOutput, String> {
        match self {
            Self::Final { content } => Ok(AgentOutput::Final { content }),
            Self::ToolCall { tool, args } => {
                if tool.trim().is_empty() {
                    Err("tool_call needs tool".to_string())
                } else {
                    Ok(AgentOutput::ToolCall { tool, args })
                }
            }
        }
    }
}
