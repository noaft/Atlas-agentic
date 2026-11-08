#![cfg(windows)]
use atlas_agentic::{
    agent_output::AgentOutput,
    tools::{register_process_tools, ToolRegistry},
};
use serde_json::{json, Value};
use std::{
    fs,
    time::{Duration, Instant},
};
fn run(tools: &ToolRegistry, name: &str, args: Value) -> Value {
    let authorization = tools.authorization(name, &args).unwrap();
    let result = tools
        .execute_authorized(
            &AgentOutput::ToolCall {
                tool: name.into(),
                args,
                message: None,
            },
            &authorization,
        )
        .unwrap();
    serde_json::from_str(&result.message).unwrap()
}
#[test]
fn windows_quick_sessions_complete_and_stopped_jobs_kill_descendants() {
    use windows_sys::Win32::{
        Foundation::{CloseHandle, WAIT_OBJECT_0},
        System::Threading::{OpenProcess, WaitForSingleObject, PROCESS_SYNCHRONIZE},
    };
    let root = tempfile::tempdir().unwrap();
    let mut tools = ToolRegistry::empty();
    register_process_tools(&mut tools, root.path());
    let fast = run(
        &tools,
        "exec_command",
        json!({"command":"echo done","yield_time_ms":1000}),
    );
    assert_eq!(fast["running"], false);
    assert_eq!(fast["exit_code"], 0);
    assert_eq!(fast["stdout"].as_str().unwrap().trim(), "done");
    let started = run(
        &tools,
        "exec_command",
        json!({"command":"powershell.exe -NoProfile -NonInteractive -Command \"Set-Content -Encoding ascii child $PID; Start-Sleep -Seconds 20\"","yield_time_ms":0}),
    );
    let deadline = Instant::now() + Duration::from_secs(10);
    let pid: u32 = loop {
        if let Ok(text) = fs::read_to_string(root.path().join("child")) {
            if let Ok(pid) = text.trim().parse() {
                break pid;
            }
        }
        let progress = run(
            &tools,
            "read_process",
            json!({"session_id":started["session_id"],"wait_ms":10}),
        );
        assert_ne!(
            progress["running"], false,
            "Child command exited before startup: {progress}"
        );
        assert!(
            Instant::now() < deadline,
            "PowerShell child did not start: {progress}"
        );
        std::thread::sleep(Duration::from_millis(10));
    };
    let child = unsafe { OpenProcess(PROCESS_SYNCHRONIZE, 0, pid) };
    assert!(!child.is_null());
    let stopped = run(
        &tools,
        "stop_process",
        json!({"session_id":started["session_id"]}),
    );
    assert_eq!(stopped["running"], false);
    assert_eq!(stopped["stopped"], true);
    let status = unsafe { WaitForSingleObject(child, 2000) };
    unsafe { CloseHandle(child) };
    assert_eq!(
        status, WAIT_OBJECT_0,
        "owned PowerShell descendant still running"
    );
}
