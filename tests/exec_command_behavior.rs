#![cfg(unix)]

use std::time::{Duration, Instant};

use atlas_agentic::tools::{ExecCommandTool, Tool};
use serde_json::{json, Value};

fn execute(tool: &ExecCommandTool, command: &str) -> Value {
    let result = tool.execute(&json!({ "command": command })).unwrap();
    serde_json::from_str(&result.message).unwrap()
}

#[cfg(not(windows))]
const SHELL_FEATURE_COMMAND: &str = "printf atlas | tr '[:lower:]' '[:upper:]'";
#[cfg(windows)]
const SHELL_FEATURE_COMMAND: &str = "echo atlas";

#[cfg(not(windows))]
const FAILURE_COMMAND: &str = "printf failure >&2; exit 7";
#[cfg(windows)]
const FAILURE_COMMAND: &str = "echo failure 1>&2 & exit /b 7";

#[cfg(not(windows))]
const WORKING_DIRECTORY_COMMAND: &str = "pwd";
#[cfg(windows)]
const WORKING_DIRECTORY_COMMAND: &str = "cd";

#[cfg(not(windows))]
const SLOW_COMMAND: &str = "sleep 1";
#[cfg(windows)]
const SLOW_COMMAND: &str = "ping -n 3 127.0.0.1 >NUL";

#[cfg(not(windows))]
const LONG_OUTPUT_COMMAND: &str = "printf 123456789";
#[cfg(windows)]
const LONG_OUTPUT_COMMAND: &str = "echo 123456789";

#[test]
fn exec_command_supports_shell_syntax_and_reports_success() {
    let workspace = tempfile::tempdir().unwrap();
    let tool = ExecCommandTool::new(workspace.path());

    let result = execute(&tool, SHELL_FEATURE_COMMAND);

    assert_eq!(result["command"], SHELL_FEATURE_COMMAND);
    assert_eq!(result["exit_code"], 0);
    assert_eq!(result["timed_out"], false);
    assert!(result["stdout"].as_str().unwrap().contains("ATLAS"));
    assert_eq!(result["stderr"], "");
}

#[test]
fn exec_command_runs_from_the_configured_workspace() {
    let workspace = tempfile::tempdir().unwrap();
    let tool = ExecCommandTool::new(workspace.path());

    let result = execute(&tool, WORKING_DIRECTORY_COMMAND);

    assert_eq!(
        result["stdout"].as_str().unwrap().trim(),
        workspace
            .path()
            .canonicalize()
            .unwrap()
            .display()
            .to_string()
    );
}

#[test]
fn exec_command_returns_nonzero_exit_and_stderr_to_the_agent() {
    let workspace = tempfile::tempdir().unwrap();
    let tool = ExecCommandTool::new(workspace.path());

    let result = execute(&tool, FAILURE_COMMAND);

    assert_eq!(result["exit_code"], 7);
    assert_eq!(result["timed_out"], false);
    assert!(result["stderr"].as_str().unwrap().contains("failure"));
}

#[test]
fn exec_command_kills_commands_that_exceed_the_timeout() {
    let workspace = tempfile::tempdir().unwrap();
    let tool = ExecCommandTool::with_limits(workspace.path(), Duration::from_millis(25), 1024);

    let result = execute(&tool, SLOW_COMMAND);

    assert_eq!(result["timed_out"], true);
    assert!(result["exit_code"].is_null());
}

#[test]
fn exec_command_timeout_includes_background_process_output_capture() {
    let workspace = tempfile::tempdir().unwrap();
    let tool = ExecCommandTool::with_limits(workspace.path(), Duration::from_millis(25), 1024);
    let started = Instant::now();

    let result = execute(&tool, "sleep 1 &");

    assert_eq!(result["timed_out"], true);
    assert!(started.elapsed() < Duration::from_millis(500));
}

#[test]
fn exec_command_returns_timeout_when_pipe_holder_escapes_process_group() {
    let workspace = tempfile::tempdir().unwrap();
    let tool = ExecCommandTool::with_limits(workspace.path(), Duration::from_millis(25), 1024);
    let started = Instant::now();
    let test_binary = std::env::current_exe().unwrap();
    let command = format!(
        "ATLAS_TEST_SETSID_PIPE_HOLDER=1 {} --exact exec_command_setsid_pipe_holder_helper --nocapture &",
        shell_quote(test_binary.to_str().unwrap())
    );

    let result = execute(&tool, &command);

    assert_eq!(result["timed_out"], true);
    assert!(result["exit_code"].is_null());
    assert!(started.elapsed() < Duration::from_millis(1_500));
}

#[test]
fn exec_command_setsid_pipe_holder_helper() {
    if std::env::var_os("ATLAS_TEST_SETSID_PIPE_HOLDER").is_none() {
        return;
    }

    // SAFETY: setsid has no pointer or memory-validity preconditions.
    assert_ne!(unsafe { libc::setsid() }, -1);
    std::thread::sleep(Duration::from_millis(1_200));
}

#[test]
fn exec_command_times_out_despite_continuous_output() {
    let workspace = tempfile::tempdir().unwrap();
    let tool = ExecCommandTool::with_limits(workspace.path(), Duration::from_millis(25), 1024);
    let started = Instant::now();

    let result = execute(&tool, "yes");

    assert_eq!(result["timed_out"], true);
    assert!(result["exit_code"].is_null());
    assert_eq!(result["stdout_truncated"], true);
    assert!(started.elapsed() < Duration::from_millis(500));
}

#[test]
fn exec_command_rejects_an_unrepresentable_timeout_before_spawning() {
    let workspace = tempfile::tempdir().unwrap();
    let marker = workspace.path().join("spawned.txt");
    let tool = ExecCommandTool::with_limits(workspace.path(), Duration::MAX, 1024);

    let error = match tool.execute(&json!({ "command": "touch spawned.txt" })) {
        Ok(_) => panic!("expected an invalid timeout error"),
        Err(error) => error,
    };

    assert_eq!(error, "exec_command timeout is too large");
    assert!(!marker.exists());
}

#[test]
fn exec_command_truncates_captured_output_at_the_configured_limit() {
    let workspace = tempfile::tempdir().unwrap();
    let tool = ExecCommandTool::with_limits(workspace.path(), Duration::from_secs(1), 4);

    let result = execute(&tool, LONG_OUTPUT_COMMAND);

    assert_eq!(result["stdout"], "1234");
    assert_eq!(result["stdout_truncated"], true);
}

#[test]
fn exec_command_rejects_an_empty_command() {
    let workspace = tempfile::tempdir().unwrap();
    let tool = ExecCommandTool::new(workspace.path());

    let error = match tool.execute(&json!({ "command": "   " })) {
        Ok(_) => panic!("expected an empty command error"),
        Err(error) => error,
    };

    assert_eq!(error, "exec_command command is required");
}

fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}
