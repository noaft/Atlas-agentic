pub mod deepseek;

use crate::agents::LocalAgentDefinition;
use crate::conversations::ConversationMessage;
use crate::media::ImageData;
use crate::settings::{AtlasSettings, EffectiveSettings, SettingField, DEFAULT_DEEPSEEK_MODEL};
use crate::tools::ToolDefinition;

use deepseek::{DeepSeekConfig, DeepSeekProvider};

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelInfo {
    pub id: String,
    pub name: String,
    pub context_window: Option<u64>,
    pub max_output_tokens: Option<u64>,
}

pub trait ModelProvider {
    fn complete(&self, request: &ModelRequest) -> Result<String, String>;

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
    let mut settings = AtlasSettings::default();
    settings.deepseek.model = definition.model.clone();
    let effective = settings.resolve_from_env()?;
    provider_from_definition_with_settings(definition, &effective)
}

pub fn provider_from_definition_with_settings(
    definition: &LocalAgentDefinition,
    settings: &EffectiveSettings,
) -> Result<Box<dyn ModelProvider>, String> {
    match definition.model_provider.as_str() {
        "deepseek" => {
            let api_key = settings
                .api_key
                .clone()
                .filter(|key| !key.trim().is_empty())
                .ok_or_else(|| "DEEPSEEK_API_KEY is required".to_string())?;
            let model = if settings.model == DEFAULT_DEEPSEEK_MODEL
                && !settings
                    .environment_overrides
                    .contains(&SettingField::Model)
            {
                definition.model.clone()
            } else {
                settings.model.clone()
            };
            Ok(Box::new(DeepSeekProvider::new(DeepSeekConfig {
                api_key,
                base_url: settings.base_url.clone(),
                model,
            })))
        }
        provider => Err(format!("unknown model provider: {provider}")),
    }
}

pub fn list_models_from_definition_with_settings(
    definition: &LocalAgentDefinition,
    settings: &EffectiveSettings,
) -> Result<Vec<ModelInfo>, String> {
    provider_from_definition_with_settings(definition, settings)?.list_models()
}
