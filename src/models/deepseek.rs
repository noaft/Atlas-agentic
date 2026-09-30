use reqwest::blocking::Client;
use serde::{Deserialize, Serialize};

use super::ModelProvider;

const DEFAULT_BASE_URL: &str = "https://api.deepseek.com";
const DEFAULT_MODEL: &str = "deepseek-flash";

pub struct DeepSeekConfig {
    pub api_key: String,
    pub base_url: String,
    pub model: String,
}

impl DeepSeekConfig {
    pub fn from_env() -> Result<Self, String> {
        let api_key = std::env::var("DEEPSEEK_API_KEY")
            .map_err(|_| "DEEPSEEK_API_KEY is required".to_string())?;
        let base_url =
            std::env::var("DEEPSEEK_BASE_URL").unwrap_or_else(|_| DEFAULT_BASE_URL.to_string());
        let model = std::env::var("DEEPSEEK_MODEL").unwrap_or_else(|_| DEFAULT_MODEL.to_string());

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
    fn complete(&self, system_prompt: &str, user_prompt: &str) -> Result<String, String> {
        let url = format!(
            "{}/chat/completions",
            self.config.base_url.trim_end_matches('/')
        );
        let request = ChatRequest {
            model: self.config.model.clone(),
            messages: vec![
                ChatMessage {
                    role: "system",
                    content: system_prompt,
                },
                ChatMessage {
                    role: "user",
                    content: user_prompt,
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
