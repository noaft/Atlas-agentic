use atlas_agentic::{
    agents::{Agent, ChatAgent},
    models::ModelProvider,
};

struct EchoModel;

impl ModelProvider for EchoModel {
    fn complete(&self, system_prompt: &str, user_prompt: &str) -> Result<String, String> {
        Ok(format!("{system_prompt} | {user_prompt}"))
    }
}

#[test]
fn chat_agent_has_conversation_role() {
    let agent = ChatAgent::new(Box::new(EchoModel));

    assert_eq!(agent.name(), "chat_agent");
    assert_eq!(
        agent.system_prompt(),
        "You are Atlas, a helpful AI companion."
    );
    assert_eq!(
        agent.process("hello").unwrap(),
        "You are Atlas, a helpful AI companion. | hello"
    );
}
