pub mod deepseek;

use crate::agents::LocalAgentDefinition;
use crate::tools::ToolDefinition;

use deepseek::{DeepSeekConfig, DeepSeekProvider};

pub trait ModelProvider {
    fn complete(&self, request: &ModelRequest) -> Result<String, String>;

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
    pub tools: &'a [ToolDefinition],
    pub temperature: Option<f32>,
    pub top_p: Option<f32>,
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
