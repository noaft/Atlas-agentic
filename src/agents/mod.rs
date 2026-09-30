mod local;

pub use local::{LocalAgent, LocalAgentDefinition};

pub trait Agent {
    fn name(&self) -> &str;
    fn system_prompt(&self) -> &str;
    fn process(
        &self,
        input: &str,
        tools: &[crate::tools::ToolDefinition],
    ) -> Result<String, String>;
    fn process_stream(
        &self,
        input: &str,
        tools: &[crate::tools::ToolDefinition],
        on_delta: &mut dyn FnMut(&str),
    ) -> Result<String, String>;
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
