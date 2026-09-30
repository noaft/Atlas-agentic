use crate::agents::AgentRegistry;

pub struct Runtime {
    agents: AgentRegistry,
}

impl Runtime {
    pub fn new(agents: AgentRegistry) -> Self {
        Self { agents }
    }

    pub fn ask(&self, prompt: &str) -> Result<String, String> {
        self.agents
            .default_agent()
            .ok_or_else(|| "no agent is registered".to_string())?
            .process(prompt)
    }
}
