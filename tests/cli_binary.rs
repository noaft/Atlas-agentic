use std::process::Command;

#[test]
fn help_command_does_not_require_deepseek_key() {
    let output = Command::new(env!("CARGO_BIN_EXE_atlas"))
        .arg("help")
        .env_remove("DEEPSEEK_API_KEY")
        .env("ATLAS_SKIP_DOTENV", "1")
        .output()
        .unwrap();

    assert!(output.status.success());
    assert!(String::from_utf8_lossy(&output.stdout).contains("Atlas Agentic"));
}

#[test]
fn ask_command_loads_deepseek_key_from_dotenv() {
    let temp_dir = tempfile::tempdir().unwrap();
    let agents_dir = temp_dir.path().join("agents");
    std::fs::create_dir(&agents_dir).unwrap();
    std::fs::write(
        agents_dir.join("atlas.toml"),
        [
            "name = \"atlas\"",
            "display_name = \"Atlas\"",
            "model_provider = \"deepseek\"",
            "model = \"deepseek-flash\"",
            "system_prompt = \"You are Atlas.\"",
            "conversation_title_prompt = \"Write a short conversation title.\"",
        ]
        .join("\n"),
    )
    .unwrap();
    std::fs::write(
        temp_dir.path().join(".env"),
        [
            "DEEPSEEK_API_KEY=fake_key_for_test",
            "DEEPSEEK_BASE_URL=http://127.0.0.1:9",
        ]
        .join("\n"),
    )
    .unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_atlas"))
        .arg("ask")
        .arg("hello atlas")
        .current_dir(temp_dir.path())
        .env_remove("DEEPSEEK_API_KEY")
        .env_remove("DEEPSEEK_BASE_URL")
        .output()
        .unwrap();

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("DeepSeek request failed"), "{stderr}");
}
