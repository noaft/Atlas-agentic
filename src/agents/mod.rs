mod local;

pub use local::{LocalAgent, LocalAgentDefinition};

use crate::conversations::ConversationMessage;
use crate::media::ImageData;

pub trait Agent {
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
