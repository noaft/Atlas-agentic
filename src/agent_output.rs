use serde::Deserialize;

pub(crate) const TOOL_MESSAGE_LIMIT: usize = 1_000;

#[derive(Debug, Clone, PartialEq)]
pub enum AgentOutput {
    Final {
        content: String,
    },
    ScheduledToolCall {
        tool: String,
        message: Option<String>,
        args: serde_json::Value,
        execution: crate::executions::protocol::ExecutionPolicy,
    },
    Execution {
        action: crate::executions::protocol::ExecutionAction,
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
        #[serde(default, deserialize_with = "present_wakeup")]
        wake_up: Option<crate::executions::protocol::ExecutionPolicy>,
        #[serde(default, deserialize_with = "legacy_execution_present")]
        execution: bool,
        #[serde(default)]
        args: serde_json::Value,
    },
    #[serde(rename = "await_execution")]
    Await {
        execution_id: Option<String>,
        execution_ids: Option<Vec<String>>,
        #[serde(default = "default_wait")]
        yield_ms: u64,
    },
    #[serde(rename = "read_execution")]
    Read {
        execution_id: String,
        tail_lines: Option<usize>,
        cursor: Option<u64>,
    },
    #[serde(rename = "set_execution_wakeup")]
    SetWakeup {
        execution_id: String,
        wake_up: crate::executions::protocol::ExecutionPolicy,
    },
    #[serde(rename = "list_executions")]
    List,
    #[serde(rename = "cancel_execution")]
    Cancel { execution_id: String },
}

impl RawAgentOutput {
    fn into_agent_output(self) -> Result<AgentOutput, String> {
        match self {
            Self::Final { content } => Ok(AgentOutput::Final { content }),
            Self::Await {
                execution_id,
                execution_ids,
                yield_ms,
            } => {
                let ids = match (execution_id, execution_ids) {
                    (Some(id), None) => vec![id],
                    (None, Some(ids)) => ids,
                    _ => return Err("Provide execution_id or execution_ids, not both".into()),
                };
                action(crate::executions::protocol::ExecutionAction::Wait { ids, yield_ms })
            }
            Self::Read {
                execution_id,
                tail_lines,
                cursor,
            } => action(crate::executions::protocol::ExecutionAction::Read {
                id: execution_id,
                tail_lines,
                cursor,
            }),
            Self::SetWakeup {
                execution_id,
                wake_up,
            } => action(crate::executions::protocol::ExecutionAction::SetWakeup {
                id: execution_id,
                wake_up,
            }),
            Self::List => action(crate::executions::protocol::ExecutionAction::List),
            Self::Cancel { execution_id } => {
                action(crate::executions::protocol::ExecutionAction::Cancel { id: execution_id })
            }

            Self::ToolCall {
                tool,
                args,
                message,
                wake_up,
                execution,
            } => {
                if execution {
                    return Err("legacy execution policy is unsupported; replace execution with top-level wake_up hard/lazy".into());
                }
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
                    if let Some(execution) = wake_up {
                        execution.validate()?;
                        return Ok(AgentOutput::ScheduledToolCall {
                            tool,
                            args,
                            message,
                            execution,
                        });
                    }
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

fn default_wait() -> u64 {
    2000
}
fn action(action: crate::executions::protocol::ExecutionAction) -> Result<AgentOutput, String> {
    action.validate()?;
    Ok(AgentOutput::Execution { action })
}

fn present_wakeup<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<crate::executions::protocol::ExecutionPolicy>, D::Error> {
    crate::executions::protocol::ExecutionPolicy::deserialize(deserializer).map(Some)
}

fn legacy_execution_present<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<bool, D::Error> {
    serde::de::IgnoredAny::deserialize(deserializer).map(|_| true)
}
