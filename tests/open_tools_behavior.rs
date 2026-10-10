use std::sync::{Arc, Mutex};

use atlas_agentic::{
    permissions::ToolRisk,
    tools::{
        application_command, file_command, CommandSpec, OpenApplicationTool, OpenFileTool,
        Platform, ProcessLauncher, Tool,
    },
};
use serde_json::json;

#[derive(Default)]
struct RecordingLauncher {
    commands: Mutex<Vec<CommandSpec>>,
    failure: Option<&'static str>,
}

impl ProcessLauncher for RecordingLauncher {
    fn launch(&self, command: &CommandSpec) -> Result<(), String> {
        self.commands.lock().unwrap().push(command.clone());
        self.failure.map_or(Ok(()), |error| Err(error.to_string()))
    }
}

#[test]
fn platform_command_specs_keep_arguments_separate() {
    assert_eq!(
        file_command(Platform::Linux, "/work/a & b.txt"),
        CommandSpec::new("xdg-open", ["/work/a & b.txt"])
    );
    assert_eq!(
        file_command(Platform::MacOs, "/work/a & b.txt"),
        CommandSpec::new("open", ["/work/a & b.txt"])
    );
    assert_eq!(
        file_command(Platform::Windows, r#"C:\work\a %PATH% ^& (b).txt"#),
        CommandSpec::new("ShellExecuteW", [r#"C:\work\a %PATH% ^& (b).txt"#])
    );
    assert_eq!(
        application_command(
            Platform::MacOs,
            "Visual Studio Code",
            &["a & b.txt".to_string(), "--reuse-window".to_string()]
        ),
        CommandSpec::new(
            "open",
            [
                "-a",
                "Visual Studio Code",
                "--args",
                "a & b.txt",
                "--reuse-window"
            ]
        )
    );
    assert_eq!(
        application_command(
            Platform::Windows,
            "code.exe",
            &["a & b.txt".to_string(), "quoted \"value\"".to_string()]
        ),
        CommandSpec::new("code.exe", ["a & b.txt", "quoted \"value\""])
    );
}

#[test]
fn open_file_validates_workspace_file_and_reports_launcher_outcome() {
    let workspace = tempfile::tempdir().unwrap();
    std::fs::write(workspace.path().join("hello world.txt"), "hello").unwrap();
    let launcher = Arc::new(RecordingLauncher::default());
    let tool = OpenFileTool::with_launcher(workspace.path(), Platform::Linux, launcher.clone());

    let result = tool.execute(&json!({"path":"hello world.txt"})).unwrap();

    assert_eq!(result.message, "Opened file: hello world.txt");
    assert_eq!(launcher.commands.lock().unwrap().len(), 1);
    assert_eq!(
        tool.risk(&json!({"path":"hello world.txt"})).unwrap(),
        ToolRisk::Launch
    );
    assert!(tool.execute(&json!({"path":"../outside.txt"})).is_err());
    assert!(tool.execute(&json!({"path":"missing.txt"})).is_err());

    let failing = OpenFileTool::with_launcher(
        workspace.path(),
        Platform::Linux,
        Arc::new(RecordingLauncher {
            commands: Mutex::new(Vec::new()),
            failure: Some("launcher unavailable"),
        }),
    );
    assert!(failing
        .execute(&json!({"path":"hello world.txt"}))
        .err()
        .unwrap()
        .contains("launcher unavailable"));
}

#[test]
fn open_application_validates_name_and_launches_asynchronously() {
    let launcher = Arc::new(RecordingLauncher::default());
    let tool = OpenApplicationTool::with_launcher(Platform::Linux, launcher.clone());

    assert!(tool.execute(&json!({"application":"  "})).is_err());
    assert_eq!(
        tool.risk(&json!({"application":"code","args":["file.txt"]}))
            .unwrap(),
        ToolRisk::Launch
    );
    let result = tool
        .execute(&json!({"application":"code","args":["file.txt"]}))
        .unwrap();

    assert_eq!(result.message, "Opened application: code");
    assert_eq!(
        launcher.commands.lock().unwrap()[0],
        CommandSpec::new("code", ["file.txt"])
    );
}
