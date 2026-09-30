mod chat;

pub use chat::ChatAgent;

pub trait Agent {
    fn name(&self) -> &'static str;
    fn system_prompt(&self) -> &'static str;
    fn process(&self, input: &str) -> Result<String, String>;
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
