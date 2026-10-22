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

impl LocalAgent {
    fn model_request<'a>(&'a self, request: &'a super::AgentRequest<'a>) -> ModelRequest<'a> {
        ModelRequest {
            memory_blocks: request.memory_blocks,
            max_output_tokens: request.max_output_tokens,
            system_prompt: self.system_prompt(),
            user_prompt: request.input,
            history: request.history,
            activity_context: request.activity_context,
            tools: request.tools,
            images: request.images,
            temperature: self.definition.temperature,
            top_p: self.definition.top_p,
        }
    }
}

impl Agent for LocalAgent {
    fn memory_policy(&self) -> Option<&crate::conversations::ContextPolicy> {
        self.definition
            .memory
            .enabled
            .then_some(&self.definition.memory)
    }
    fn summary_prompt(&self) -> Option<&str> {
        self.definition.conversation_summary_prompt.as_deref()
    }
    fn model_name(&self) -> &str {
        self.model.model_name().unwrap_or(&self.definition.model)
    }
    fn model_limits(&self) -> Result<Option<crate::models::ModelLimits>, String> {
        self.model.model_limits()
    }
    fn estimate_request(
        &self,
        request: &super::AgentRequest,
    ) -> Result<Option<crate::models::TokenEstimate>, String> {
        self.model.estimate_request(&self.model_request(request))
    }
    fn estimate_summary(
        &self,
        request: &crate::models::SummaryRequest,
    ) -> Result<Option<crate::models::TokenEstimate>, String> {
        self.model.estimate_request(&ModelRequest {
            memory_blocks: &[],
            max_output_tokens: Some(request.max_output_tokens),
            system_prompt: request.system_prompt,
            user_prompt: request.content,
            history: &[],
            activity_context: None,
            tools: &[],
            images: &[],
            temperature: None,
            top_p: None,
        })
    }
    fn summarize(
        &self,
        request: &crate::models::SummaryRequest,
    ) -> Result<crate::models::SummaryResponse, String> {
        crate::diagnostics::run(
            "model.summary",
            serde_json::json!({"model": self.model_name()}),
            || self.model.summarize(request),
        )
    }
    fn process_request_with_usage(
        &self,
        request: &super::AgentRequest,
        on_delta: &mut dyn FnMut(&str),
    ) -> Result<crate::models::ModelResponse, String> {
        crate::diagnostics::run(
            "model.chat",
            serde_json::json!({"model": self.model_name()}),
            || {
                self.model
                    .stream_with_usage(&self.model_request(request), on_delta)
            },
        )
    }

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
            memory_blocks: &[],
            max_output_tokens: None,
            system_prompt: self.system_prompt(),
            user_prompt: input,
            history,
            activity_context: None,
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
                memory_blocks: &[],
                max_output_tokens: None,
                system_prompt: self.system_prompt(),
                user_prompt: input,
                history,
                activity_context: None,
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
                memory_blocks: &[],
                max_output_tokens: None,
                system_prompt: self.system_prompt(),
                user_prompt: input,
                history,
                activity_context: None,
                tools,
                images,
                temperature: self.definition.temperature,
                top_p: self.definition.top_p,
            },
            on_delta,
        )
    }

    fn process_stream_with_context_and_images(
        &self,
        input: &str,
        history: &[ConversationMessage],
        tools: &[ToolDefinition],
        activity_context: Option<&str>,
        images: &[ImageData],
        on_delta: &mut dyn FnMut(&str),
    ) -> Result<String, String> {
        self.model.stream(
            &ModelRequest {
                memory_blocks: &[],
                max_output_tokens: None,
                system_prompt: self.system_prompt(),
                user_prompt: input,
                history,
                activity_context,
                tools,
                images,
                temperature: self.definition.temperature,
                top_p: self.definition.top_p,
            },
            on_delta,
        )
    }

    fn process_stream_with_usage(
        &self,
        input: &str,
        history: &[ConversationMessage],
        tools: &[ToolDefinition],
        activity_context: Option<&str>,
        images: &[ImageData],
        on_delta: &mut dyn FnMut(&str),
    ) -> Result<crate::models::ModelResponse, String> {
        self.model.stream_with_usage(
            &ModelRequest {
                memory_blocks: &[],
                max_output_tokens: None,
                system_prompt: self.system_prompt(),
                user_prompt: input,
                history,
                activity_context,
                tools,
                images,
                temperature: self.definition.temperature,
                top_p: self.definition.top_p,
            },
            on_delta,
        )
    }

    fn generate_conversation_title(&self, first_input: &str) -> Result<String, String> {
        crate::diagnostics::run(
            "model.title",
            serde_json::json!({"model": self.model_name()}),
            || {
                self.model.complete(&ModelRequest {
                    memory_blocks: &[],
                    max_output_tokens: None,
                    system_prompt: &self.definition.conversation_title_prompt,
                    user_prompt: first_input,
                    history: &[],
                    activity_context: None,
                    tools: &[],
                    images: &[],
                    temperature: self.definition.temperature,
                    top_p: self.definition.top_p,
                })
            },
        )
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
    pub conversation_summary_prompt: Option<String>,
    pub memory: crate::conversations::ContextPolicy,
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
    conversation_summary_prompt: Option<String>,
    memory: Option<crate::conversations::ContextPolicy>,
}

impl RawLocalAgentDefinition {
    fn validate(self) -> Result<LocalAgentDefinition, String> {
        let memory = self.memory.unwrap_or_default();
        memory.validate()?;
        if memory.enabled
            && self
                .conversation_summary_prompt
                .as_ref()
                .is_none_or(|p| p.trim().is_empty())
        {
            return Err("Memory-enabled agent needs conversation_summary_prompt".into());
        }
        Ok(LocalAgentDefinition {
            memory,
            conversation_summary_prompt: self.conversation_summary_prompt,
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
