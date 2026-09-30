use atlas_agentic::{
    agents::{AgentRegistry, ChatAgent},
    handle_command, AppResult, Runtime,
};

struct FakeModel;

impl atlas_agentic::models::ModelProvider for FakeModel {
    fn complete(&self, system_prompt: &str, user_prompt: &str) -> Result<String, String> {
        Ok(format!("{system_prompt} | {user_prompt}"))
    }
}

fn test_runtime() -> Runtime {
    Runtime::new(AgentRegistry::new(vec![Box::new(ChatAgent::new(
        Box::new(FakeModel),
    ))]))
}

#[test]
fn shows_help_when_no_command_is_given() {
    let args: Vec<&str> = vec![];
    let runtime = test_runtime();
    let result = handle_command(args, &runtime);

    assert_eq!(result.unwrap(), AppResult::Help);
}

#[test]
fn ask_command_returns_agent_response() {
    let runtime = test_runtime();
    let result = handle_command(["ask", "hello atlas"], &runtime);

    assert_eq!(
        result.unwrap(),
        AppResult::Message("You are Atlas, a helpful AI companion. | hello atlas".to_string())
    );
}
