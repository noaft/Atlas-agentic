use crate::models::ModelProvider;

use super::Agent;

pub struct ChatAgent {
    model: Box<dyn ModelProvider>,
}

impl ChatAgent {
    pub fn new(model: Box<dyn ModelProvider>) -> Self {
        Self { model }
    }
}

impl Agent for ChatAgent {
    fn name(&self) -> &'static str {
        "chat_agent"
    }

    fn system_prompt(&self) -> &'static str {
        "You are Atlas, a helpful AI companion."
    }

    fn process(&self, input: &str) -> Result<String, String> {
        self.model.complete(self.system_prompt(), input)
    }
}
