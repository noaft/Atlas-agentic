use std::io::Read;

use base64::{engine::general_purpose::STANDARD, Engine as _};
use reqwest::blocking::Client;
use serde::{Deserialize, Serialize};

use crate::conversations::MessageRole;
use crate::media::ImageData;

use super::{ModelInfo, ModelProvider, ModelRequest};

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
    fn validate_images(&self, images: &[ImageData]) -> Result<(), String> {
        images.iter().try_for_each(validate_image)
    }

    fn complete(&self, request: &ModelRequest) -> Result<String, String> {
        let payload = self.chat_request(request, false)?;
        let response = self
            .client
            .post(self.chat_url())
            .bearer_auth(&self.config.api_key)
            .json(&payload)
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
        let payload = self.chat_request(request, true)?;
        let mut response = self
            .client
            .post(self.chat_url())
            .bearer_auth(&self.config.api_key)
            .json(&payload)
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

    fn list_models(&self) -> Result<Vec<ModelInfo>, String> {
        let response = self
            .client
            .get(self.models_url())
            .bearer_auth(&self.config.api_key)
            .send()
            .map_err(|error| format!("DeepSeek model list request failed: {error}"))?;

        if !response.status().is_success() {
            return Err(format!(
                "DeepSeek model list returned HTTP {}",
                response.status()
            ));
        }

        let response = response
            .text()
            .map_err(|error| format!("DeepSeek model list response could not be read: {error}"))?;
        parse_model_list(&response)
    }
}

impl DeepSeekProvider {
    fn chat_url(&self) -> String {
        format!(
            "{}/chat/completions",
            self.config.base_url.trim_end_matches('/')
        )
    }

    fn models_url(&self) -> String {
        format!("{}/models", self.config.base_url.trim_end_matches('/'))
    }

    fn chat_request(&self, request: &ModelRequest, stream: bool) -> Result<ChatRequest, String> {
        let system_prompt = render_model_instructions(request);
        let mut messages = Vec::with_capacity(request.history.len() + 2);
        messages.push(ChatMessage {
            role: "system",
            content: ChatContent::Text(system_prompt),
        });
        messages.extend(request.history.iter().map(|message| ChatMessage {
            role: match message.role {
                MessageRole::User => "user",
                MessageRole::Assistant => "assistant",
            },
            content: ChatContent::Text(message.content.clone()),
        }));
        let user_content = if request.images.is_empty() {
            ChatContent::Text(request.user_prompt.to_string())
        } else {
            self.validate_images(request.images)?;
            let mut parts = Vec::with_capacity(request.images.len() + 1);
            parts.push(ChatContentPart::Text {
                text: request.user_prompt.to_string(),
            });
            for image in request.images {
                parts.push(ChatContentPart::ImageUrl {
                    image_url: ImageUrl {
                        url: format!(
                            "data:{};base64,{}",
                            image.mime_type,
                            STANDARD.encode(&image.bytes)
                        ),
                        detail: image.detail,
                    },
                });
            }
            ChatContent::Parts(parts)
        };
        messages.push(ChatMessage {
            role: "user",
            content: user_content,
        });

        Ok(ChatRequest {
            model: self.config.model.clone(),
            messages,
            stream,
            response_format: (!request.tools.is_empty()).then_some(ResponseFormat {
                output_type: "json_object",
            }),
            temperature: request.temperature,
            top_p: request.top_p,
        })
    }
}

fn validate_image(image: &ImageData) -> Result<(), String> {
    if image.mime_type != "image/png" {
        return Err(format!(
            "DeepSeek image input requires image/png, got {}",
            image.mime_type
        ));
    }
    if image.width == 0 || image.height == 0 {
        return Err("DeepSeek image input dimensions must be nonzero".to_string());
    }
    if image.width > 8192 || image.height > 8192 {
        return Err("DeepSeek image input dimensions must not exceed 8192 pixels".to_string());
    }
    if image.bytes.len() > 32 * 1024 * 1024 {
        return Err("DeepSeek image input must not exceed 32 MiB".to_string());
    }
    Ok(())
}

pub(crate) fn render_model_instructions(request: &ModelRequest) -> String {
    let mut prompt = request.system_prompt.to_string();
    if let Some(activity_context) = request
        .activity_context
        .filter(|value| !value.trim().is_empty())
    {
        prompt.push_str("\n\nAtlas memory from prior successful tool activity:\n");
        prompt.push_str(activity_context);
    }
    if request.tools.is_empty() {
        return prompt;
    }
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
    response_format: Option<ResponseFormat>,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    top_p: Option<f32>,
}

#[derive(Serialize)]
struct ResponseFormat {
    #[serde(rename = "type")]
    output_type: &'static str,
}

#[derive(Serialize)]
struct ChatMessage {
    role: &'static str,
    content: ChatContent,
}

#[derive(Serialize)]
#[serde(untagged)]
enum ChatContent {
    Text(String),
    Parts(Vec<ChatContentPart>),
}

#[derive(Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ChatContentPart {
    Text { text: String },
    ImageUrl { image_url: ImageUrl },
}

#[derive(Serialize)]
struct ImageUrl {
    url: String,
    detail: crate::media::ImageDetail,
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
struct ModelListResponse {
    data: Vec<DeepSeekModel>,
}

#[derive(Deserialize)]
struct DeepSeekModel {
    id: String,
    name: Option<String>,
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

fn parse_model_list(response: &str) -> Result<Vec<ModelInfo>, String> {
    let response: ModelListResponse = serde_json::from_str(response)
        .map_err(|error| format!("DeepSeek model list was invalid: {error}"))?;
    Ok(response
        .data
        .into_iter()
        .map(|model| ModelInfo {
            name: model
                .name
                .filter(|name| !name.trim().is_empty())
                .unwrap_or_else(|| model.id.clone()),
            id: model.id,
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use std::{
        io::{Read, Write},
        net::TcpListener,
        thread,
    };

    use chrono::{TimeZone, Utc};
    use serde_json::json;

    use crate::{
        conversations::{ConversationMessage, MessageRole},
        media::{ImageData, ImageDetail},
        models::{
            deepseek::{
                parse_model_list, parse_stream_delta, render_model_instructions, DeepSeekConfig,
                DeepSeekProvider,
            },
            ModelProvider, ModelRequest,
        },
        tools::ToolDefinition,
    };

    fn provider() -> DeepSeekProvider {
        DeepSeekProvider::new(DeepSeekConfig {
            api_key: "test-key".to_string(),
            base_url: "http://localhost".to_string(),
            model: "test-model".to_string(),
        })
    }

    fn request<'a>(images: &'a [ImageData]) -> ModelRequest<'a> {
        ModelRequest {
            system_prompt: "You are Atlas.",
            user_prompt: "inspect this screen",
            history: &[],
            activity_context: None,
            tools: &[],
            images,
            temperature: None,
            top_p: None,
        }
    }

    fn png(bytes: Vec<u8>, width: u32, height: u32) -> ImageData {
        ImageData {
            mime_type: "image/png".to_string(),
            bytes,
            width,
            height,
            detail: ImageDetail::Original,
        }
    }

    #[test]
    fn chat_request_serializes_text_only_content_as_a_string() {
        let payload =
            serde_json::to_value(provider().chat_request(&request(&[]), false).unwrap()).unwrap();

        assert_eq!(
            payload["messages"][1]["content"],
            json!("inspect this screen")
        );
    }

    #[test]
    fn chat_request_serializes_png_after_text_as_original_image_input() {
        let images = vec![png(vec![1, 2, 3], 2, 1)];
        let payload =
            serde_json::to_value(provider().chat_request(&request(&images), false).unwrap())
                .unwrap();

        assert_eq!(
            payload["messages"][1]["content"],
            json!([
                {"type": "text", "text": "inspect this screen"},
                {"type": "image_url", "image_url": {
                    "url": "data:image/png;base64,AQID",
                    "detail": "original"
                }}
            ])
        );
    }

    #[test]
    fn chat_request_accepts_exact_image_limits() {
        let images = vec![png(vec![0; 32 * 1024 * 1024], 8192, 8192)];

        assert!(provider().chat_request(&request(&images), false).is_ok());
    }

    #[test]
    fn chat_request_rejects_oversized_image_dimensions_and_bytes() {
        for image in [
            png(vec![1], 8193, 1),
            png(vec![1], 1, 8193),
            png(vec![0; 32 * 1024 * 1024 + 1], 1, 1),
            png(vec![1], 0, 1),
            ImageData {
                mime_type: "image/jpeg".to_string(),
                bytes: vec![1],
                width: 1,
                height: 1,
                detail: ImageDetail::Low,
            },
        ] {
            assert!(provider().chat_request(&request(&[image]), false).is_err());
        }
    }

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
            activity_context: None,
            tools: &tools,
            images: &[],
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
    fn activity_context_is_a_labeled_system_section_not_a_fake_message() {
        let provider = provider();
        let mut request = request(&[]);
        request.activity_context =
            Some("Recent Atlas activity (facts from earlier successful turns):\n- searched files");

        let payload =
            serde_json::to_value(provider.chat_request(&request, false).unwrap()).unwrap();
        let messages = payload["messages"].as_array().unwrap();

        assert_eq!(messages.len(), 2);
        let system = messages[0]["content"].as_str().unwrap();
        assert!(system.contains("Atlas memory from prior successful tool activity:"));
        assert!(system.contains("searched files"));
        assert!(!system.contains("RAW_PRIVATE_DETAIL"));
        assert_eq!(messages[1]["role"], "user");
    }

    #[test]
    fn empty_activity_context_adds_no_memory_section() {
        let prompt = render_model_instructions(&request(&[]));

        assert!(!prompt.contains("Atlas memory from prior successful tool activity:"));
    }

    #[test]
    fn tool_request_enables_provider_json_output() {
        let provider = DeepSeekProvider::new(DeepSeekConfig {
            api_key: "test-key".to_string(),
            base_url: "http://localhost".to_string(),
            model: "test-model".to_string(),
        });
        let tools = vec![ToolDefinition {
            name: "read_file".to_string(),
            description: "Read a file.".to_string(),
            input_schema: json!({"type": "object"}),
        }];
        let request = ModelRequest {
            system_prompt: "You are Atlas.",
            user_prompt: "Read README.md.",
            history: &[],
            activity_context: None,
            tools: &tools,
            images: &[],
            temperature: None,
            top_p: None,
        };

        let payload = serde_json::to_value(provider.chat_request(&request, true).unwrap()).unwrap();

        assert_eq!(payload["response_format"], json!({"type": "json_object"}));
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
            activity_context: None,
            tools: &[],
            images: &[],
            temperature: None,
            top_p: None,
        };

        let request =
            serde_json::to_value(provider.chat_request(&request, false).unwrap()).unwrap();
        let messages = request["messages"].as_array().unwrap();

        assert_eq!(
            messages,
            &vec![
                json!({"role": "system", "content": "You are Atlas."}),
                json!({"role": "user", "content": "Earlier question"}),
                json!({"role": "assistant", "content": "Earlier answer"}),
                json!({"role": "user", "content": "Newest question"}),
            ]
        );
    }

    #[test]
    fn parses_stream_delta_line() {
        let delta =
            parse_stream_delta(r#"data: {"choices":[{"delta":{"content":"hello"}}]}"#).unwrap();

        assert_eq!(delta, Some("hello".to_string()));
    }

    #[test]
    fn parses_model_list_and_uses_the_id_when_the_display_name_is_missing() {
        let models = parse_model_list(
            r#"{
                "object": "list",
                "data": [
                    {"id": "deepseek-flash", "object": "model", "owned_by": "deepseek", "name": "DeepSeek Flash"},
                    {"id": "deepseek-v4-pro", "object": "model", "owned_by": "deepseek"}
                ]
            }"#,
        )
        .unwrap();

        assert_eq!(models[0].id, "deepseek-flash");
        assert_eq!(models[0].name, "DeepSeek Flash");
        assert_eq!(models[1].id, "deepseek-v4-pro");
        assert_eq!(models[1].name, "deepseek-v4-pro");
    }

    #[test]
    fn lists_models_with_bearer_auth_from_the_configured_base_url() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0_u8; 4096];
            let read = stream.read(&mut request).unwrap();
            let request = String::from_utf8_lossy(&request[..read]);
            assert!(request.starts_with("GET /models HTTP/1.1\r\n"), "{request}");
            assert!(
                request
                    .to_ascii_lowercase()
                    .contains("authorization: bearer test-key\r\n"),
                "{request}"
            );
            let body = r#"{"object":"list","data":[{"id":"deepseek-flash","object":"model","owned_by":"deepseek","name":"DeepSeek Flash"}]}"#;
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            )
            .unwrap();
        });
        let provider = DeepSeekProvider::new(DeepSeekConfig {
            api_key: "test-key".to_string(),
            base_url: format!("http://{address}"),
            model: "deepseek-flash".to_string(),
        });

        let models = provider.list_models().unwrap();

        server.join().unwrap();
        assert_eq!(models[0].id, "deepseek-flash");
        assert_eq!(models[0].name, "DeepSeek Flash");
    }
}
