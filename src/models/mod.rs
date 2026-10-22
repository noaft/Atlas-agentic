mod anthropic;
pub mod budget;
mod compatible;
pub mod discovery;
mod gemini;
mod native;
mod openai;
pub(crate) mod prompt;
pub mod sse;
pub(crate) mod usage;
pub use budget::{ModelLimits, TokenEstimate};
pub mod deepseek;

use crate::agents::LocalAgentDefinition;
use crate::conversations::ConversationMessage;
use crate::media::ImageData;
use crate::settings::{AtlasSettings, EffectiveSettings};
use crate::tools::ToolDefinition;

use deepseek::{DeepSeekConfig, DeepSeekProvider};

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelInfo {
    pub source: Option<crate::settings::ModelSource>,
    pub input_token_limit: Option<u64>,
    pub capabilities: crate::settings::ProviderCapabilities,
    pub id: String,
    pub name: String,
    pub context_window: Option<u64>,
    pub max_output_tokens: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelUsage {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<crate::settings::ModelSource>,
    pub model: String,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub total_tokens: u64,
}

pub struct ModelResponse {
    pub content: String,
    pub usage: Option<ModelUsage>,
}

pub struct SummaryRequest<'a> {
    pub system_prompt: &'a str,
    pub content: &'a str,
    pub max_output_tokens: u64,
}
pub struct SummaryResponse {
    pub content: String,
    pub usage: Option<ModelUsage>,
    pub finish_reason: String,
}
pub trait ModelProvider {
    fn model_name(&self) -> Option<&str> {
        None
    }
    fn model_limits(&self) -> Result<Option<ModelLimits>, String> {
        Ok(None)
    }
    fn estimate_request(&self, _request: &ModelRequest) -> Result<Option<TokenEstimate>, String> {
        Ok(None)
    }
    fn summarize(&self, _request: &SummaryRequest) -> Result<SummaryResponse, String> {
        Err("Model provider does not support compaction".into())
    }

    fn complete(&self, request: &ModelRequest) -> Result<String, String>;

    fn stream_with_usage(
        &self,
        request: &ModelRequest,
        on_delta: &mut dyn FnMut(&str),
    ) -> Result<ModelResponse, String> {
        self.stream(request, on_delta).map(|content| ModelResponse {
            content,
            usage: None,
        })
    }

    fn validate_images(&self, images: &[ImageData]) -> Result<(), String> {
        if images.is_empty() {
            Ok(())
        } else {
            Err("model provider does not support image input".to_string())
        }
    }

    fn list_models(&self) -> Result<Vec<ModelInfo>, String> {
        Err("model provider does not support model discovery".to_string())
    }

    fn stream(
        &self,
        request: &ModelRequest,
        on_delta: &mut dyn FnMut(&str),
    ) -> Result<String, String> {
        let response = self.complete(request)?;
        on_delta(&response);
        Ok(response)
    }
}

pub struct ModelRequest<'a> {
    pub memory_blocks: &'a [crate::conversations::MemoryBlock],
    pub max_output_tokens: Option<u64>,
    pub system_prompt: &'a str,
    pub user_prompt: &'a str,
    pub history: &'a [ConversationMessage],
    pub activity_context: Option<&'a str>,
    pub tools: &'a [ToolDefinition],
    pub images: &'a [ImageData],
    pub temperature: Option<f32>,
    pub top_p: Option<f32>,
}

pub fn provider_from_definition(
    definition: &LocalAgentDefinition,
) -> Result<Box<dyn ModelProvider>, String> {
    let settings = AtlasSettings::default();
    let effective =
        settings.resolve_for(&definition.model_provider, &definition.model, |name| {
            std::env::var_os(name)
        })?;
    provider_from_definition_with_settings(definition, &effective)
}

pub fn provider_from_definition_with_settings(
    definition: &LocalAgentDefinition,
    settings: &EffectiveSettings,
) -> Result<Box<dyn ModelProvider>, String> {
    match settings.provider.as_str() {
        "deepseek" => {
            let api_key = settings
                .api_key
                .clone()
                .filter(|key| !key.trim().is_empty())
                .ok_or_else(|| "DEEPSEEK_API_KEY is required".to_string())?;
            let model = if settings.model_from_agent
                && definition.model_provider == settings.provider.as_str()
            {
                definition.model.clone()
            } else {
                settings.model.clone()
            };
            if model.trim().is_empty() {
                return Err("Select a DeepSeek model in Settings".into());
            }
            Ok(Box::new(
                DeepSeekProvider::new(DeepSeekConfig {
                    api_key,
                    base_url: settings.base_url.clone(),
                    model: model.clone(),
                })
                .with_limits(settings.model_limits.get(&model).copied()),
            ))
        }
        _ => Ok(Box::new(native::NativeProvider::new(settings.clone())?)),
    }
}

pub fn list_models_from_definition_with_settings(
    definition: &LocalAgentDefinition,
    settings: &EffectiveSettings,
) -> Result<Vec<ModelInfo>, String> {
    let _ = definition;
    discovery::discover_models(settings)
}
