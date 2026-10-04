use atlas_agentic::{
    agents::{Agent, LocalAgent, LocalAgentDefinition},
    models::{ModelProvider, ModelRequest},
};

struct EchoModel;

impl ModelProvider for EchoModel {
    fn complete(&self, request: &ModelRequest) -> Result<String, String> {
        Ok(format!(
            "{} | {}",
            request.system_prompt, request.user_prompt
        ))
    }
}

#[test]
fn local_agent_uses_definition_from_file() {
    let definition = LocalAgentDefinition::from_toml(
        r#"
name = "atlas"
display_name = "Atlas"
model_provider = "fake"
model = "deepseek-flash"
temperature = 0.3
top_p = 0.8
system_prompt = "You are Atlas, the default local agent."
"#,
    )
    .unwrap();
    let agent = LocalAgent::new(definition, Box::new(EchoModel));

    assert_eq!(agent.name(), "atlas");
    assert_eq!(agent.definition().temperature, Some(0.3));
    assert_eq!(agent.definition().top_p, Some(0.8));
    assert_eq!(
        agent.system_prompt(),
        "You are Atlas, the default local agent."
    );
    assert_eq!(
        agent.process("hello", &[], &[]).unwrap(),
        "You are Atlas, the default local agent. | hello"
    );
}

#[test]
fn rejects_agent_definition_with_invalid_temperature() {
    let result = LocalAgentDefinition::from_toml(
        r#"
name = "atlas"
display_name = "Atlas"
model_provider = "fake"
model = "fake"
temperature = 3.0
system_prompt = "Invalid temperature."
"#,
    );

    assert_eq!(result.unwrap_err(), "temperature must be between 0 and 2");
}

#[test]
fn rejects_agent_definition_without_name() {
    let result = LocalAgentDefinition::from_toml(
        r#"
display_name = "Broken"
model_provider = "fake"
model = "deepseek-flash"
system_prompt = "Missing name."
"#,
    );

    assert_eq!(result.unwrap_err(), "agent definition needs name");
}

#[test]
fn rejects_agent_definition_without_model_provider() {
    let result = LocalAgentDefinition::from_toml(
        r#"
name = "atlas"
display_name = "Atlas"
model = "deepseek-flash"
system_prompt = "Missing provider."
"#,
    );

    assert_eq!(result.unwrap_err(), "agent definition needs model_provider");
}
