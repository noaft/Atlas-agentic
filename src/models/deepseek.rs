use reqwest::blocking::Client;
use serde::{Deserialize, Serialize};

use super::{ModelProvider, ModelRequest};

const DEFAULT_BASE_URL: &str = "https://api.deepseek.com";
pub struct DeepSeekConfig {
    pub api_key: String,
    pub base_url: String,
    pub model: String,
}

impl DeepSeekConfig {
    pub fn from_env(default_model: &str) -> Result<Self, String> {
        let api_key = std::env::var("DEEPSEEK_API_KEY")
            .map_err(|_| "DEEPSEEK_API_KEY is required".to_string())?;
        let base_url =
            std::env::var("DEEPSEEK_BASE_URL").unwrap_or_else(|_| DEFAULT_BASE_URL.to_string());
        let model = std::env::var("DEEPSEEK_MODEL").unwrap_or_else(|_| default_model.to_string());

        Ok(Self {
            api_key,
            base_url,
            model,
        })
    }
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
        let url = format!(
            "{}/chat/completions",
            self.config.base_url.trim_end_matches('/')
        );
        let system_prompt = render_model_instructions(request);
        let request = ChatRequest {
            model: self.config.model.clone(),
            messages: vec![
                ChatMessage {
                    role: "system",
                    content: &system_prompt,
                },
                ChatMessage {
                    role: "user",
                    content: request.user_prompt,
                },
            ],
            stream: false,
        };

        let response = self
            .client
            .post(url)
            .bearer_auth(&self.config.api_key)
            .json(&request)
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
}

pub(crate) fn render_model_instructions(request: &ModelRequest) -> String {
    if request.tools.is_empty() {
        return request.system_prompt.to_string();
    }

    let mut prompt = request.system_prompt.to_string();
    prompt.push_str("\n\nAtlas runtime tools are available.");
    prompt.push_str("\nYou must respond with exactly one JSON object and no markdown.");
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
struct ChatRequest<'a> {
    model: String,
    messages: Vec<ChatMessage<'a>>,
    stream: bool,
}

#[derive(Serialize)]
struct ChatMessage<'a> {
    role: &'a str,
    content: &'a str,
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

#[cfg(test)]
mod tests {
    use serde_json::json;

    use crate::{
        models::{deepseek::render_model_instructions, ModelRequest},
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
            tools: &tools,
        };

        let prompt = render_model_instructions(&request);

        assert!(prompt.contains("exactly one JSON object and no markdown"));
        assert!(prompt.contains(r#"{"type":"final","content":"your answer"}"#));
        assert!(prompt.contains(r#"{"type":"tool_call","tool":"tool_name","args":{}}"#));
        assert!(prompt.contains("Use a tool name exactly as listed below"));
        assert!(prompt.contains("create_file"));
    }
}
