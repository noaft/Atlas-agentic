pub mod deepseek;

use crate::agents::LocalAgentDefinition;
use crate::tools::ToolDefinition;

use deepseek::{DeepSeekConfig, DeepSeekProvider};

pub trait ModelProvider {
    fn complete(&self, request: &ModelRequest) -> Result<String, String>;
}

pub struct ModelRequest<'a> {
    pub system_prompt: &'a str,
    pub user_prompt: &'a str,
    pub tools: &'a [ToolDefinition],
}

pub fn provider_from_definition(
    definition: &LocalAgentDefinition,
) -> Result<Box<dyn ModelProvider>, String> {
    match definition.model_provider.as_str() {
        "deepseek" => Ok(Box::new(DeepSeekProvider::new(DeepSeekConfig::from_env(
            &definition.model,
        )?))),
        provider => Err(format!("unknown model provider: {provider}")),
    }
}
