use atlas_agentic::{
    agents::{AgentRegistry, LocalAgent, LocalAgentDefinition},
    handle_command, AppResult, Runtime,
};

struct FakeModel;

impl atlas_agentic::models::ModelProvider for FakeModel {
    fn complete(&self, request: &atlas_agentic::models::ModelRequest) -> Result<String, String> {
        Ok(format!(
            "{} | {}",
            request.system_prompt, request.user_prompt
        ))
    }
}

fn test_runtime() -> Runtime {
    let definition = LocalAgentDefinition::from_toml(
        r#"
name = "atlas"
display_name = "Atlas"
model_provider = "fake"
model = "deepseek-flash"
system_prompt = "You are Atlas, the default local agent."
"#,
    )
    .unwrap();

    Runtime::new(AgentRegistry::new(vec![Box::new(LocalAgent::new(
        definition,
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
        AppResult::Message("You are Atlas, the default local agent. | hello atlas".to_string())
    );
}
