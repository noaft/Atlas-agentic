use std::io::Read;

use reqwest::blocking::Client;
use serde::{Deserialize, Serialize};

use crate::conversations::MessageRole;

use super::{ModelProvider, ModelRequest};

pub struct DeepSeekConfig {
    pub api_key: String,
    pub base_url: String,
    pub model: String,
}

pub struct DeepSeekProvider {
    config: DeepSeekConfig,
    client: Client,
}

impl DeepSeekProvider {
    pub fn new(config: DeepSeekConfig) -> Self {
        Self {
            config,
            client: Client::new(),
        }
    }
}

impl ModelProvider for DeepSeekProvider {
    fn complete(&self, request: &ModelRequest) -> Result<String, String> {
        let response = self
            .client
            .post(self.chat_url())
            .bearer_auth(&self.config.api_key)
            .json(&self.chat_request(request, false))
            .send()
            .map_err(|error| format!("DeepSeek request failed: {error}"))?;

        if !response.status().is_success() {
            return Err(format!("DeepSeek returned HTTP {}", response.status()));
        }

        let response: ChatResponse = response
            .json()
            .map_err(|error| format!("DeepSeek response was invalid: {error}"))?;

        response
            .choices
            .into_iter()
            .next()
            .map(|choice| choice.message.content)
            .ok_or_else(|| "DeepSeek returned no choices".to_string())
    }

    fn stream(
        &self,
        request: &ModelRequest,
        on_delta: &mut dyn FnMut(&str),
    ) -> Result<String, String> {
        let mut response = self
            .client
            .post(self.chat_url())
            .bearer_auth(&self.config.api_key)
            .json(&self.chat_request(request, true))
            .send()
            .map_err(|error| format!("DeepSeek request failed: {error}"))?;

        if !response.status().is_success() {
            return Err(format!("DeepSeek returned HTTP {}", response.status()));
        }

        let mut buffer = String::new();
        let mut full_response = String::new();
        let mut bytes = [0_u8; 8192];

        loop {
            let read = response
                .read(&mut bytes)
                .map_err(|error| format!("DeepSeek stream failed: {error}"))?;
            if read == 0 {
                break;
            }

            buffer.push_str(&String::from_utf8_lossy(&bytes[..read]));
            while let Some(newline) = buffer.find('\n') {
                let line = buffer[..newline].trim().to_string();
                buffer = buffer[newline + 1..].to_string();

                if line.is_empty() {
                    continue;
                }
                if line == "data: [DONE]" {
                    return Ok(full_response);
                }
                if let Some(delta) = parse_stream_delta(&line)? {
                    on_delta(&delta);
                    full_response.push_str(&delta);
                }
            }
        }

        Ok(full_response)
    }
}

impl DeepSeekProvider {
    fn chat_url(&self) -> String {
        format!(
            "{}/chat/completions",
            self.config.base_url.trim_end_matches('/')
        )
    }

    fn chat_request(&self, request: &ModelRequest, stream: bool) -> ChatRequest {
        let system_prompt = render_model_instructions(request);
        let mut messages = Vec::with_capacity(request.history.len() + 2);
        messages.push(ChatMessage {
            role: "system",
            content: system_prompt,
        });
        messages.extend(request.history.iter().map(|message| ChatMessage {
            role: match message.role {
                MessageRole::User => "user",
                MessageRole::Assistant => "assistant",
            },
            content: message.content.clone(),
        }));
        messages.push(ChatMessage {
            role: "user",
            content: request.user_prompt.to_string(),
        });

        ChatRequest {
            model: self.config.model.clone(),
            messages,
            stream,
            temperature: request.temperature,
            top_p: request.top_p,
        }
    }
}

pub(crate) fn render_model_instructions(request: &ModelRequest) -> String {
    if request.tools.is_empty() {
        return request.system_prompt.to_string();
    }

    let mut prompt = request.system_prompt.to_string();
    prompt.push_str("\n\nAtlas runtime tools are available.");
    prompt.push_str("\nYou must respond with exactly one JSON object and no markdown.");
    prompt.push_str("\nThe type field must be the first field in the JSON object.");
    prompt.push_str("\nIf no tool is needed, use only this format:");
    prompt.push_str("\n{\"type\":\"final\",\"content\":\"your answer\"}");
    prompt.push_str("\nIf a tool is needed, use only this format:");
    prompt.push_str("\n{\"type\":\"tool_call\",\"tool\":\"tool_name\",\"args\":{}}");
    prompt.push_str(
        "\nUse a tool name exactly as listed below. Put all tool parameters inside args.",
    );
    prompt.push_str("\nAvailable tools:\n");

    for tool in request.tools {
        prompt.push_str("- ");
        prompt.push_str(&tool.name);
        prompt.push_str(": ");
        prompt.push_str(&tool.description);
        prompt.push_str("\n  input_schema: ");
        prompt.push_str(&tool.input_schema.to_string());
        prompt.push('\n');
    }

    prompt
}

#[derive(Serialize)]
struct ChatRequest {
    model: String,
    messages: Vec<ChatMessage>,
    stream: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    top_p: Option<f32>,
}

#[derive(Serialize)]
struct ChatMessage {
    role: &'static str,
    content: String,
}

#[derive(Deserialize)]
struct ChatResponse {
    choices: Vec<ChatChoice>,
}

#[derive(Deserialize)]
struct ChatChoice {
    message: AssistantMessage,
}

#[derive(Deserialize)]
struct AssistantMessage {
    content: String,
}

#[derive(Deserialize)]
struct ChatStreamResponse {
    choices: Vec<ChatStreamChoice>,
}

#[derive(Deserialize)]
struct ChatStreamChoice {
    delta: ChatStreamDelta,
}

#[derive(Deserialize)]
struct ChatStreamDelta {
    content: Option<String>,
}

fn parse_stream_delta(line: &str) -> Result<Option<String>, String> {
    let Some(data) = line.strip_prefix("data: ") else {
        return Ok(None);
    };
    let response: ChatStreamResponse = serde_json::from_str(data)
        .map_err(|error| format!("DeepSeek stream response was invalid: {error}"))?;

    Ok(response
        .choices
        .into_iter()
        .next()
        .and_then(|choice| choice.delta.content))
}

#[cfg(test)]
mod tests {
    use chrono::{TimeZone, Utc};
    use serde_json::json;

    use crate::{
        conversations::{ConversationMessage, MessageRole},
        models::{
            deepseek::{
                parse_stream_delta, render_model_instructions, DeepSeekConfig, DeepSeekProvider,
            },
            ModelRequest,
        },
        tools::ToolDefinition,
    };

    #[test]
    fn tool_instructions_require_exact_json_response_format() {
        let tools = vec![ToolDefinition {
            name: "create_file".to_string(),
            description: "Create a file.".to_string(),
            input_schema: json!({
                "type": "object",
                "required": ["path", "content"]
            }),
        }];
        let request = ModelRequest {
            system_prompt: "You are Atlas.",
            user_prompt: "Create a file.",
            history: &[],
            tools: &tools,
            temperature: Some(0.2),
            top_p: Some(0.9),
        };

        let prompt = render_model_instructions(&request);

        assert!(prompt.contains("exactly one JSON object and no markdown"));
        assert!(prompt.contains("type field must be the first field"));
        assert!(prompt.contains(r#"{"type":"final","content":"your answer"}"#));
        assert!(prompt.contains(r#"{"type":"tool_call","tool":"tool_name","args":{}}"#));
        assert!(prompt.contains("Use a tool name exactly as listed below"));
        assert!(prompt.contains("create_file"));
    }

    #[test]
    fn chat_request_orders_system_history_and_current_user_input() {
        let history = vec![
            ConversationMessage {
                role: MessageRole::User,
                content: "Earlier question".to_string(),
                created_at: Utc.with_ymd_and_hms(2026, 10, 4, 12, 0, 0).unwrap(),
            },
            ConversationMessage {
                role: MessageRole::Assistant,
                content: "Earlier answer".to_string(),
                created_at: Utc.with_ymd_and_hms(2026, 10, 4, 12, 0, 1).unwrap(),
            },
        ];
        let provider = DeepSeekProvider::new(DeepSeekConfig {
            api_key: "test-key".to_string(),
            base_url: "http://localhost".to_string(),
            model: "test-model".to_string(),
        });
        let request = ModelRequest {
            system_prompt: "You are Atlas.",
            user_prompt: "Newest question",
            history: &history,
            tools: &[],
            temperature: None,
            top_p: None,
        };

        let request = provider.chat_request(&request, false);
        let messages = request
            .messages
            .iter()
            .map(|message| (message.role, message.content.as_str()))
            .collect::<Vec<_>>();

        assert_eq!(
            messages,
            vec![
                ("system", "You are Atlas."),
                ("user", "Earlier question"),
                ("assistant", "Earlier answer"),
                ("user", "Newest question"),
            ]
        );
    }

    #[test]
    fn parses_stream_delta_line() {
        let delta =
            parse_stream_delta(r#"data: {"choices":[{"delta":{"content":"hello"}}]}"#).unwrap();

        assert_eq!(delta, Some("hello".to_string()));
    }
}
