use serde::Deserialize;

use crate::{
    models::{ModelProvider, ModelRequest},
    tools::ToolDefinition,
};

use super::Agent;

pub struct LocalAgent {
    definition: LocalAgentDefinition,
    model: Box<dyn ModelProvider>,
}

impl LocalAgent {
    pub fn new(definition: LocalAgentDefinition, model: Box<dyn ModelProvider>) -> Self {
        Self { definition, model }
    }
}

impl Agent for LocalAgent {
    fn name(&self) -> &str {
        &self.definition.name
    }

    fn system_prompt(&self) -> &str {
        &self.definition.system_prompt
    }

    fn process(&self, input: &str, tools: &[ToolDefinition]) -> Result<String, String> {
        self.model.complete(&ModelRequest {
            system_prompt: self.system_prompt(),
            user_prompt: input,
            tools,
        })
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct LocalAgentDefinition {
    pub name: String,
    pub display_name: String,
    pub model_provider: String,
    pub model: String,
    pub system_prompt: String,
}

impl LocalAgentDefinition {
    pub fn from_toml(input: &str) -> Result<Self, String> {
        let definition: RawLocalAgentDefinition =
            toml::from_str(input).map_err(|error| format!("invalid agent TOML: {error}"))?;
        definition.validate()
    }

    pub fn from_file(path: impl AsRef<std::path::Path>) -> Result<Self, String> {
        let path = path.as_ref();
        let input = std::fs::read_to_string(path)
            .map_err(|error| format!("failed to read {}: {error}", path.display()))?;
        Self::from_toml(&input)
    }
}

#[derive(Deserialize)]
struct RawLocalAgentDefinition {
    name: Option<String>,
    display_name: Option<String>,
    model_provider: Option<String>,
    model: Option<String>,
    system_prompt: Option<String>,
}

impl RawLocalAgentDefinition {
    fn validate(self) -> Result<LocalAgentDefinition, String> {
        Ok(LocalAgentDefinition {
            name: required_field(self.name, "name")?,
            display_name: required_field(self.display_name, "display_name")?,
            model_provider: required_field(self.model_provider, "model_provider")?,
            model: required_field(self.model, "model")?,
            system_prompt: required_field(self.system_prompt, "system_prompt")?,
        })
    }
}

fn required_field(value: Option<String>, name: &str) -> Result<String, String> {
    let value = value.unwrap_or_default();
    if value.trim().is_empty() {
        Err(format!("agent definition needs {name}"))
    } else {
        Ok(value)
    }
}
