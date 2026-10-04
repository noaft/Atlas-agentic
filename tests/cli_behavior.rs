use atlas_agentic::{handle_command, AppResult};

#[test]
fn shows_help_when_no_command_is_given() {
    let args: Vec<&str> = vec![];
    let result = handle_command(args);

    assert_eq!(result.unwrap(), AppResult::Help);
}

#[test]
fn ask_command_uses_streaming_by_default() {
    let result = handle_command(["ask", "hello atlas"]);

    assert_eq!(
        result.unwrap(),
        AppResult::Stream {
            prompt: "hello atlas".to_string()
        }
    );
}

#[test]
fn ask_stream_command_returns_stream_request() {
    let result = handle_command(["ask", "--stream", "hello atlas"]);

    assert_eq!(
        result.unwrap(),
        AppResult::Stream {
            prompt: "hello atlas".to_string()
        }
    );
}
