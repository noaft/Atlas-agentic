use atlas_agentic::{agents::LocalAgentDefinition, models::provider_from_definition};

#[test]
fn provider_factory_rejects_unknown_provider() {
    let definition = LocalAgentDefinition::from_toml(
        r#"
name = "atlas"
display_name = "Atlas"
model_provider = "unknown"
model = "some-model"
system_prompt = "You are Atlas."
"#,
    )
    .unwrap();

    let result = provider_from_definition(&definition);

    match result {
        Ok(_) => panic!("expected unknown provider error"),
        Err(error) => assert_eq!(error, "unknown model provider: unknown"),
    }
}

#[test]
fn provider_factory_uses_deepseek_provider_name() {
    let definition = LocalAgentDefinition::from_toml(
        r#"
name = "atlas"
display_name = "Atlas"
model_provider = "deepseek"
model = "deepseek-flash"
system_prompt = "You are Atlas."
"#,
    )
    .unwrap();

    let provider = provider_from_definition(&definition);

    match provider {
        Ok(_) => panic!("expected missing DeepSeek key error"),
        Err(error) => assert_eq!(error, "DEEPSEEK_API_KEY is required"),
    }
}
