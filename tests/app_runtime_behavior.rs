use atlas_agentic::{
    app::{build_runtime, build_runtime_with_settings},
    settings::{AtlasSettings, EffectiveSettings},
};
use tempfile::TempDir;

#[test]
fn build_runtime_reports_the_requested_missing_agent_file() {
    let workspace = TempDir::new().unwrap();
    let missing = workspace.path().join("config/missing-agent.toml");

    let Err(error) = build_runtime(workspace.path(), &missing) else {
        panic!("missing agent file unexpectedly built a runtime");
    };

    assert!(error.contains(&missing.display().to_string()), "{error}");
}

#[test]
fn build_runtime_accepts_a_saved_api_key_without_environment_access() {
    let workspace = TempDir::new().unwrap();
    let agent_file = workspace.path().join("atlas.toml");
    std::fs::write(&agent_file, include_str!("../agents/atlas.toml")).unwrap();
    let mut saved = AtlasSettings::default();
    saved.deepseek.api_key = Some("saved-key".to_string());
    let effective = saved.resolve_with(|_| None).unwrap();

    let runtime = build_runtime_with_settings(workspace.path(), &agent_file, &effective);

    assert!(runtime.is_ok());
}

#[test]
fn missing_agent_file_error_wins_over_missing_provider_credentials() {
    let workspace = TempDir::new().unwrap();
    let missing = workspace.path().join("missing-agent.toml");
    let effective = EffectiveSettings::default();

    let Err(error) = build_runtime_with_settings(workspace.path(), &missing, &effective) else {
        panic!("missing agent file unexpectedly built a runtime");
    };

    assert!(error.contains(&missing.display().to_string()), "{error}");
    assert!(!error.contains("DEEPSEEK_API_KEY"), "{error}");
}
