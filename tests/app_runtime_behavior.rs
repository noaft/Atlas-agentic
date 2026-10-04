use atlas_agentic::app::build_runtime;
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
