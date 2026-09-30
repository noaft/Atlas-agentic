use atlas_agentic::{handle_command, AppResult};

#[test]
fn shows_help_when_no_command_is_given() {
    let args: Vec<&str> = vec![];
    let result = handle_command(args);

    assert_eq!(result.unwrap(), AppResult::Help);
}

#[test]
fn ask_command_returns_agent_response() {
    let result = handle_command(["ask", "hello atlas"]);

    assert_eq!(
        result.unwrap(),
        AppResult::Message("Atlas heard: hello atlas".to_string())
    );
}
