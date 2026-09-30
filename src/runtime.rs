use crate::{agent_output::AgentOutput, agents::AgentRegistry, tools::ToolRegistry};

pub struct Runtime {
    agents: AgentRegistry,
    tools: ToolRegistry,
}

impl Runtime {
    pub fn new(agents: AgentRegistry) -> Self {
        Self {
            agents,
            tools: ToolRegistry::empty(),
        }
    }

    pub fn with_tools(mut self, tools: ToolRegistry) -> Self {
        self.tools = tools;
        self
    }

    pub fn ask(&self, prompt: &str) -> Result<String, String> {
        let response = self
            .agents
            .default_agent()
            .ok_or_else(|| "no agent is registered".to_string())?
            .process(prompt, &self.tools.definitions())?;
        let output = AgentOutput::parse(&response)?;

        self.tools.execute(&output).map(|result| result.message)
    }

    pub fn ask_stream(
        &self,
        prompt: &str,
        on_delta: &mut dyn FnMut(&str),
    ) -> Result<String, String> {
        let response = self
            .agents
            .default_agent()
            .ok_or_else(|| "no agent is registered".to_string())?
            .process_stream(prompt, &self.tools.definitions(), on_delta)?;
        let output = AgentOutput::parse(&response)?;

        self.tools.execute(&output).map(|result| result.message)
    }
}
