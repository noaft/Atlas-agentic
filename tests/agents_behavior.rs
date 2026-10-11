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
max_tool_calls = 12
system_prompt = "You are Atlas, the default local agent."
conversation_title_prompt = "Write a short conversation title."
"#,
    )
    .unwrap();
    let agent = LocalAgent::new(definition, Box::new(EchoModel));

    assert_eq!(agent.name(), "atlas");
    assert_eq!(agent.definition().temperature, Some(0.3));
    assert_eq!(agent.definition().top_p, Some(0.8));
    assert_eq!(agent.definition().max_tool_calls, Some(12));
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
fn tool_call_limit_is_optional_but_must_be_positive() {
    let unlimited = LocalAgentDefinition::from_toml(
        r#"
name = "atlas"
display_name = "Atlas"
model_provider = "fake"
model = "fake"
system_prompt = "No configured tool-call limit."
conversation_title_prompt = "Write a short conversation title."
"#,
    )
    .unwrap();
    assert_eq!(unlimited.max_tool_calls, None);

    let invalid = LocalAgentDefinition::from_toml(
        r#"
name = "atlas"
display_name = "Atlas"
model_provider = "fake"
model = "fake"
max_tool_calls = 0
system_prompt = "Invalid tool-call limit."
conversation_title_prompt = "Write a short conversation title."
"#,
    );
    assert_eq!(
        invalid.unwrap_err(),
        "max_tool_calls must be a positive integer when configured"
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
conversation_title_prompt = "Write a short conversation title."
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
conversation_title_prompt = "Write a short conversation title."
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
conversation_title_prompt = "Write a short conversation title."
"#,
    );

    assert_eq!(result.unwrap_err(), "agent definition needs model_provider");
}

#[test]
fn rejects_agent_definition_without_conversation_title_prompt() {
    let result = LocalAgentDefinition::from_toml(
        r#"
name = "atlas"
display_name = "Atlas"
model_provider = "fake"
model = "deepseek-flash"
system_prompt = "Missing title prompt."
"#,
    );

    assert_eq!(
        result.unwrap_err(),
        "agent definition needs conversation_title_prompt"
    );
}

#[test]
fn default_atlas_prompt_guides_general_computer_work_without_embedding_tool_protocol() {
    let definition = LocalAgentDefinition::from_file("agents/atlas.toml").unwrap();
    assert_eq!(definition.max_tool_calls, Some(32));
    let prompt = definition.system_prompt.to_lowercase();

    for required in [
        "conversation",
        "files",
        "applications",
        "computer",
        "shell",
        "starting context",
        "os permissions",
        "inspect",
        "relevant context",
        "runtime-provided tools",
        "coding",
        "preserve unrelated",
        "verify",
        "blockers",
        "markdown",
    ] {
        assert!(
            prompt.contains(required),
            "default Atlas prompt is missing `{required}` guidance"
        );
    }

    let prompt_tokens = prompt
        .split(|character: char| !character.is_ascii_alphanumeric() && character != '_')
        .collect::<Vec<_>>();
    for tool_name in ["pwd", "ls", "read_file", "create_file", "exec_command"] {
        assert!(
            !prompt_tokens.contains(&tool_name),
            "default Atlas prompt must not name runtime tool `{tool_name}`"
        );
    }
    assert!(
        !prompt.contains('{') && !prompt.contains('}'),
        "default Atlas prompt must not embed tool protocol JSON"
    );
}

#[test]
fn readme_documents_the_complete_default_toolset_and_permission_modes() {
    let readme = std::fs::read_to_string("README.md").unwrap();
    for tool_name in [
        "current_time",
        "pwd",
        "ls",
        "read_file",
        "search_files",
        "create_file",
        "create_directory",
        "copy_file",
        "move_file",
        "delete_file",
        "open_file",
        "open_application",
        "exec_command",
        "mouse_move",
        "mouse_click",
        "mouse_scroll",
        "type_text",
        "key_press",
        "get_ui_state",
    ] {
        assert!(
            readme.contains(&format!("`{tool_name}`")),
            "README is missing default tool `{tool_name}`"
        );
    }
    for mode in ["Ask", "Workspace", "Full access"] {
        assert!(
            readme.contains(&format!("**{mode}**")),
            "README is missing permission mode `{mode}`"
        );
    }
}
