mod local;

pub use local::{LocalAgent, LocalAgentDefinition};

use crate::conversations::ConversationMessage;
use crate::media::ImageData;

pub struct AgentRequest<'a> {
    pub input: &'a str,
    pub history: &'a [ConversationMessage],
    pub memory_blocks: &'a [crate::conversations::MemoryBlock],
    pub tools: &'a [crate::tools::ToolDefinition],
    pub activity_context: Option<&'a str>,
    pub images: &'a [ImageData],
    pub max_output_tokens: Option<u64>,
}
pub trait Agent {
    fn memory_policy(&self) -> Option<&crate::conversations::ContextPolicy> {
        None
    }
    fn summary_prompt(&self) -> Option<&str> {
        None
    }
    fn model_name(&self) -> &str {
        self.name()
    }
    fn model_limits(&self) -> Result<Option<crate::models::ModelLimits>, String> {
        Ok(None)
    }
    fn estimate_request(
        &self,
        _request: &AgentRequest,
    ) -> Result<Option<crate::models::TokenEstimate>, String> {
        Ok(None)
    }
    fn estimate_summary(
        &self,
        _request: &crate::models::SummaryRequest,
    ) -> Result<Option<crate::models::TokenEstimate>, String> {
        Ok(None)
    }
    fn summarize(
        &self,
        _request: &crate::models::SummaryRequest,
    ) -> Result<crate::models::SummaryResponse, String> {
        Err("Agent does not support compaction".into())
    }
    fn process_request_with_usage(
        &self,
        request: &AgentRequest,
        on_delta: &mut dyn FnMut(&str),
    ) -> Result<crate::models::ModelResponse, String> {
        if !request.memory_blocks.is_empty() {
            return Err("Agent does not support saved memory".into());
        }
        self.process_stream_with_usage(
            request.input,
            request.history,
            request.tools,
            request.activity_context,
            request.images,
            on_delta,
        )
    }

    fn name(&self) -> &str;
    fn system_prompt(&self) -> &str;
    fn max_tool_calls(&self) -> Option<usize> {
        None
    }
    fn validate_images(&self, images: &[ImageData]) -> Result<(), String> {
        if images.is_empty() {
            Ok(())
        } else {
            Err("model provider does not support image input".to_string())
        }
    }
    fn process(
        &self,
        input: &str,
        history: &[ConversationMessage],
        tools: &[crate::tools::ToolDefinition],
    ) -> Result<String, String>;
    fn process_stream(
        &self,
        input: &str,
        history: &[ConversationMessage],
        tools: &[crate::tools::ToolDefinition],
        on_delta: &mut dyn FnMut(&str),
    ) -> Result<String, String>;
    fn process_stream_with_images(
        &self,
        input: &str,
        history: &[ConversationMessage],
        tools: &[crate::tools::ToolDefinition],
        images: &[ImageData],
        on_delta: &mut dyn FnMut(&str),
    ) -> Result<String, String> {
        if images.is_empty() {
            self.process_stream(input, history, tools, on_delta)
        } else {
            Err("model provider does not support image input".to_string())
        }
    }
    fn process_stream_with_context_and_images(
        &self,
        input: &str,
        history: &[ConversationMessage],
        tools: &[crate::tools::ToolDefinition],
        _activity_context: Option<&str>,
        images: &[ImageData],
        on_delta: &mut dyn FnMut(&str),
    ) -> Result<String, String> {
        self.process_stream_with_images(input, history, tools, images, on_delta)
    }
    fn process_stream_with_usage(
        &self,
        input: &str,
        history: &[ConversationMessage],
        tools: &[crate::tools::ToolDefinition],
        activity_context: Option<&str>,
        images: &[ImageData],
        on_delta: &mut dyn FnMut(&str),
    ) -> Result<crate::models::ModelResponse, String> {
        self.process_stream_with_context_and_images(
            input,
            history,
            tools,
            activity_context,
            images,
            on_delta,
        )
        .map(|content| crate::models::ModelResponse {
            content,
            usage: None,
        })
    }
    fn generate_conversation_title(&self, first_input: &str) -> Result<String, String>;
}

pub struct AgentRegistry {
    agents: Vec<Box<dyn Agent>>,
}

impl AgentRegistry {
    pub fn new(agents: Vec<Box<dyn Agent>>) -> Self {
        Self { agents }
    }

    pub fn default_agent(&self) -> Option<&dyn Agent> {
        self.agents.first().map(|agent| agent.as_ref())
    }
}
