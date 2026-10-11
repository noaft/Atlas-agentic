use serde::Deserialize;

use crate::{
    conversations::ConversationMessage,
    media::ImageData,
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

    pub fn definition(&self) -> &LocalAgentDefinition {
        &self.definition
    }
}

impl Agent for LocalAgent {
    fn name(&self) -> &str {
        &self.definition.name
    }

    fn system_prompt(&self) -> &str {
        &self.definition.system_prompt
    }

    fn max_tool_calls(&self) -> Option<usize> {
        self.definition.max_tool_calls
    }

    fn validate_images(&self, images: &[ImageData]) -> Result<(), String> {
        self.model.validate_images(images)
    }

    fn process(
        &self,
        input: &str,
        history: &[ConversationMessage],
        tools: &[ToolDefinition],
    ) -> Result<String, String> {
        self.model.complete(&ModelRequest {
            system_prompt: self.system_prompt(),
            user_prompt: input,
            history,
            tools,
            images: &[],
            temperature: self.definition.temperature,
            top_p: self.definition.top_p,
        })
    }

    fn process_stream(
        &self,
        input: &str,
        history: &[ConversationMessage],
        tools: &[ToolDefinition],
        on_delta: &mut dyn FnMut(&str),
    ) -> Result<String, String> {
        self.model.stream(
            &ModelRequest {
                system_prompt: self.system_prompt(),
                user_prompt: input,
                history,
                tools,
                images: &[],
                temperature: self.definition.temperature,
                top_p: self.definition.top_p,
            },
            on_delta,
        )
    }

    fn process_stream_with_images(
        &self,
        input: &str,
        history: &[ConversationMessage],
        tools: &[ToolDefinition],
        images: &[ImageData],
        on_delta: &mut dyn FnMut(&str),
    ) -> Result<String, String> {
        self.model.stream(
            &ModelRequest {
                system_prompt: self.system_prompt(),
                user_prompt: input,
                history,
                tools,
                images,
                temperature: self.definition.temperature,
                top_p: self.definition.top_p,
            },
            on_delta,
        )
    }

    fn generate_conversation_title(&self, first_input: &str) -> Result<String, String> {
        self.model.complete(&ModelRequest {
            system_prompt: &self.definition.conversation_title_prompt,
            user_prompt: first_input,
            history: &[],
            tools: &[],
            images: &[],
            temperature: self.definition.temperature,
            top_p: self.definition.top_p,
        })
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct LocalAgentDefinition {
    pub name: String,
    pub display_name: String,
    pub model_provider: String,
    pub model: String,
    pub temperature: Option<f32>,
    pub top_p: Option<f32>,
    pub max_tool_calls: Option<usize>,
    pub system_prompt: String,
    pub conversation_title_prompt: String,
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
    temperature: Option<f32>,
    top_p: Option<f32>,
    max_tool_calls: Option<usize>,
    system_prompt: Option<String>,
    conversation_title_prompt: Option<String>,
}

impl RawLocalAgentDefinition {
    fn validate(self) -> Result<LocalAgentDefinition, String> {
        Ok(LocalAgentDefinition {
            name: required_field(self.name, "name")?,
            display_name: required_field(self.display_name, "display_name")?,
            model_provider: required_field(self.model_provider, "model_provider")?,
            model: required_field(self.model, "model")?,
            temperature: validate_range(self.temperature, "temperature", 0.0, 2.0)?,
            top_p: validate_range(self.top_p, "top_p", 0.0, 1.0)?,
            max_tool_calls: validate_tool_call_limit(self.max_tool_calls)?,
            system_prompt: required_field(self.system_prompt, "system_prompt")?,
            conversation_title_prompt: required_field(
                self.conversation_title_prompt,
                "conversation_title_prompt",
            )?,
        })
    }
}

fn validate_tool_call_limit(value: Option<usize>) -> Result<Option<usize>, String> {
    if value == Some(0) {
        return Err("max_tool_calls must be a positive integer when configured".to_string());
    }
    Ok(value)
}

fn validate_range(
    value: Option<f32>,
    name: &str,
    minimum: f32,
    maximum: f32,
) -> Result<Option<f32>, String> {
    if let Some(value) = value {
        if !(minimum..=maximum).contains(&value) {
            return Err(format!(
                "{name} must be between {minimum:.0} and {maximum:.0}"
            ));
        }
    }
    Ok(value)
}

fn required_field(value: Option<String>, name: &str) -> Result<String, String> {
    let value = value.unwrap_or_default();
    if value.trim().is_empty() {
        Err(format!("agent definition needs {name}"))
    } else {
        Ok(value)
    }
}
