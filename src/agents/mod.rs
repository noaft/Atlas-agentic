mod local;

pub use local::{LocalAgent, LocalAgentDefinition};

use crate::conversations::ConversationMessage;

pub trait Agent {
    fn name(&self) -> &str;
    fn system_prompt(&self) -> &str;
    fn max_tool_calls(&self) -> Option<usize> {
        None
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
