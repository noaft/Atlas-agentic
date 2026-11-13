#![cfg(unix)]
use atlas_agentic::{
    agent_output::AgentOutput,
    permissions::ToolRisk,
    tools::{register_process_tools, ToolRegistry},
};
use serde_json::{json, Value};
use std::{
    fs,
    path::Path,
    time::{Duration, Instant},
};
fn tools(path: &Path) -> ToolRegistry {
    let mut tools = ToolRegistry::empty();
    register_process_tools(&mut tools, path);
    tools
}
fn run(tools: &ToolRegistry, name: &str, args: Value) -> Result<Value, String> {
    let auth = tools.authorization(name, &args)?;
    let result = tools.execute_authorized(
        &AgentOutput::ToolCall {
            tool: name.into(),
            args,
            message: None,
        },
        &auth,
    )?;
    serde_json::from_str(&result.message).map_err(|e| e.to_string())
}
fn finish(tools: &ToolRegistry, id: &str) -> Value {
    let deadline = Instant::now() + Duration::from_secs(3);
    let mut stdout = String::new();
    let mut stderr = String::new();
    loop {
        let mut result = run(
            tools,
            "read_process",
            json!({"session_id":id,"wait_ms":100}),
        )
        .unwrap();
        stdout.push_str(result["stdout"].as_str().unwrap());
        stderr.push_str(result["stderr"].as_str().unwrap());
        if result["running"] == false {
            result["stdout"] = stdout.into();
            result["stderr"] = stderr.into();
            return result;
        }
        assert!(Instant::now() < deadline, "process did not complete");
    }
}
#[test]
fn quick_commands_complete_and_slow_commands_yield_with_incremental_output() {
    let root = tempfile::tempdir().unwrap();
    let tools = tools(root.path());
    let fast = run(
        &tools,
        "exec_command",
        json!({"command":"printf quick","yield_time_ms":1000}),
    )
    .unwrap();
    assert_eq!(fast["exit_code"], 0);
    assert_eq!(fast["stdout"], "quick");
    assert_eq!(fast["running"], false);
    let first = run(
        &tools,
        "exec_command",
        json!({"command":"printf first; sleep .2; printf second","yield_time_ms":30}),
    )
    .unwrap();
    assert_eq!(first["running"], true);
    let id = first["session_id"].as_str().unwrap();
    let last = finish(&tools, id);
    assert_eq!(
        format!(
            "{}{}",
            first["stdout"].as_str().unwrap(),
            last["stdout"].as_str().unwrap()
        ),
        "firstsecond"
    );
    assert_eq!(last["exit_code"], 0);
    let again = run(&tools, "read_process", json!({"session_id":id,"wait_ms":0})).unwrap();
    assert_eq!(again["stdout"], "");
}
#[test]
fn stop_and_timeout_clean_up_owned_descendants() {
    let root = tempfile::tempdir().unwrap();
    let tools = tools(root.path());
    let started = run(
        &tools,
        "exec_command",
        json!({"command":"sleep 20 & echo $! > child; wait","yield_time_ms":30}),
    )
    .unwrap();
    let id = started["session_id"].as_str().unwrap();
    let deadline = Instant::now() + Duration::from_secs(2);
    while !root.path().join("child").exists() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(
        tools
            .authorization("stop_process", &json!({"session_id":id}))
            .unwrap()
            .risk(),
        ToolRisk::Command
    );
    let stopped = run(&tools, "stop_process", json!({"session_id":id})).unwrap();
    assert_eq!(stopped["running"], false);
    assert_eq!(stopped["stopped"], true);
    assert!(stopped["exit_code"].is_null());
    let pid: i32 = fs::read_to_string(root.path().join("child"))
        .unwrap()
        .trim()
        .parse()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(2);
    while unsafe { libc::kill(pid, 0) } == 0 && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
    // A reparented child can briefly remain a zombie; it must no longer execute.
    if unsafe { libc::kill(pid, 0) } == 0 {
        assert!(fs::read_to_string(format!("/proc/{pid}/stat"))
            .unwrap_or_default()
            .contains(") Z "));
    }
    let timed = run(
        &tools,
        "exec_command",
        json!({"command":"sleep 2","yield_time_ms":0,"timeout_ms":40}),
    )
    .unwrap();
    let end = finish(&tools, timed["session_id"].as_str().unwrap());
    assert_eq!(end["timed_out"], true);
    assert_eq!(end["stopped"], false);
}
#[test]
fn session_ids_are_runtime_owned_and_drop_cleans_up() {
    let root = tempfile::tempdir().unwrap();
    let owner = tools(root.path());
    let other = tools(root.path());
    let started = run(
        &owner,
        "exec_command",
        json!({"command":"sleep .5; touch leaked","yield_time_ms":0}),
    )
    .unwrap();
    let id = started["session_id"].as_str().unwrap();
    assert!(run(&other, "read_process", json!({"session_id":id})).is_err());
    assert!(run(&other, "stop_process", json!({"session_id":id})).is_err());
    drop(owner);
    std::thread::sleep(Duration::from_millis(550));
    assert!(!root.path().join("leaked").exists());
}
#[test]
fn invalid_session_arguments_cannot_launch_and_read_only_poll_needs_no_confirmation() {
    let root = tempfile::tempdir().unwrap();
    let tools = tools(root.path());
    for extra in [
        json!({"yield_time_ms":10001}),
        json!({"yield_time_ms":-1}),
        json!({"yield_time_ms":0,"timeout_ms":0}),
        json!({"yield_time_ms":0,"timeout_ms":600001}),
    ] {
        let mut input = json!({"command":"touch launched"});
        input
            .as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        assert!(run(&tools, "exec_command", input).is_err());
    }
    assert!(!root.path().join("launched").exists());
    let started = run(
        &tools,
        "exec_command",
        json!({"command":"sleep .1","yield_time_ms":0}),
    )
    .unwrap();
    let id = started["session_id"].as_str().unwrap();
    assert_eq!(
        tools
            .authorization("read_process", &json!({"session_id":id}))
            .unwrap()
            .risk(),
        ToolRisk::ReadOnly
    );
    assert!(run(
        &tools,
        "read_process",
        json!({"session_id":id,"wait_ms":10001})
    )
    .is_err());
}

#[test]
fn session_limits_are_enforced_before_launch_and_output_is_bounded() {
    let root = tempfile::tempdir().unwrap();
    let owner = tools(root.path());
    for _ in 0..8 {
        run(
            &owner,
            "exec_command",
            json!({"command":"sleep 20","yield_time_ms":0}),
        )
        .unwrap();
    }
    assert!(run(
        &owner,
        "exec_command",
        json!({"command":"touch ninth","yield_time_ms":0})
    )
    .is_err());
    assert!(!root.path().join("ninth").exists());
    drop(owner);
    let owner = tools(root.path());
    let result=run(&owner,"exec_command",json!({"command":"head -c 200000 /dev/zero | tr '\\0' x; printf error >&2","yield_time_ms":1000})).unwrap();
    assert_eq!(result["running"], false);
    assert_eq!(result["stdout"].as_str().unwrap().len(), 65536);
    assert_eq!(result["stdout_truncated"], true);
    assert_eq!(result["stderr"], "error");
}

#[test]
fn normal_completion_does_not_leave_redirected_children_running() {
    let root = tempfile::tempdir().unwrap();
    let tools = tools(root.path());
    let result = run(
        &tools,
        "exec_command",
        json!({"command":"(sleep .3; touch leaked) >/dev/null 2>&1 &","yield_time_ms":1000}),
    )
    .unwrap();
    assert_eq!(result["running"], false);
    assert_eq!(result["exit_code"], 0);
    drop(tools);
    std::thread::sleep(Duration::from_millis(400));
    assert!(!root.path().join("leaked").exists());
}

#[test]
fn continuously_ready_output_does_not_block_stop() {
    let root = tempfile::tempdir().unwrap();
    let owner = tools(root.path());
    let started = run(
        &owner,
        "exec_command",
        json!({"command":"yes x","yield_time_ms":30,"timeout_ms":3000}),
    )
    .unwrap();
    let stopped = run(
        &owner,
        "stop_process",
        json!({"session_id":started["session_id"]}),
    )
    .unwrap();
    assert_eq!(stopped["running"], false);
    assert_eq!(stopped["stopped"], true);
}

#[test]
fn tail_reads_last_lines_per_stream_without_consuming_incremental_output() {
    let root = tempfile::tempdir().unwrap();
    let tools = tools(root.path());
    let first = run(&tools, "exec_command", json!({
        "command":"printf 'one\\r\\ntwo\\nthree'; printf 'warn1\\nwarn2\\n' >&2", "yield_time_ms":0
    })).unwrap();
    let id = first["session_id"].as_str().unwrap();
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        let snapshot = run(
            &tools,
            "read_process",
            json!({"session_id":id,"cursor":{"stdout":0,"stderr":0},"wait_ms":100}),
        )
        .unwrap();
        if snapshot["running"] == false {
            break;
        }
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(5));
    }
    let tail = run(
        &tools,
        "read_process",
        json!({"session_id":id,"tail_lines":1,"wait_ms":0}),
    )
    .unwrap();
    assert_eq!(tail["stdout"], "three");
    assert_eq!(tail["stderr"], "warn2\n");
    assert_eq!(tail["tail_lines"], 1);
    let other_root = tempfile::tempdir().unwrap();
    let mut other = ToolRegistry::empty();
    register_process_tools(&mut other, other_root.path());
    assert!(other
        .authorization("read_process", &json!({"session_id":id,"tail_lines":1}))
        .is_err());
    assert_eq!(tail["stdout_view"]["start_line"], 3);
    assert_eq!(tail["stdout_view"]["end_line"], 3);
    assert_eq!(tail["stdout_view"]["total_lines"], 3);
    assert_eq!(tail["stdout_view"]["lines_after"], 0);
    assert_eq!(tail["stdout_view"]["start_byte"], 9);
    assert_eq!(tail["stdout_view"]["end_byte"], 14);
    assert_eq!(tail["stdout_view"]["has_more"], false);
    assert_eq!(tail["stdout_truncated"], false);
    let incremental = run(&tools, "read_process", json!({"session_id":id,"wait_ms":0})).unwrap();
    assert_eq!(
        format!(
            "{}{}",
            first["stdout"].as_str().unwrap(),
            incremental["stdout"].as_str().unwrap()
        ),
        "one\r\ntwo\nthree"
    );
    assert_eq!(
        format!(
            "{}{}",
            first["stderr"].as_str().unwrap(),
            incremental["stderr"].as_str().unwrap()
        ),
        "warn1\nwarn2\n"
    );
    assert_eq!(
        run(
            &tools,
            "read_process",
            json!({"session_id":id,"tail_lines":2,"wait_ms":0})
        )
        .unwrap()["stdout"],
        "two\nthree"
    );
}

#[test]
fn tail_arguments_are_validated_before_execution() {
    let root = tempfile::tempdir().unwrap();
    let tools = tools(root.path());
    let first = run(
        &tools,
        "exec_command",
        json!({"command":"printf ok","yield_time_ms":1000}),
    )
    .unwrap();
    let id = first["session_id"].as_str().unwrap();
    for extra in [
        json!({"tail_lines":0}),
        json!({"tail_lines":1001}),
        json!({"tail_lines":-1}),
        json!({"tail_lines":1,"cursor":{"stdout":0,"stderr":0}}),
    ] {
        let mut args = extra;
        args["session_id"] = id.into();
        assert!(
            tools.authorization("read_process", &args).is_err(),
            "accepted {args}"
        );
    }
    let args = json!({"session_id":id,"tail_lines":1000,"wait_ms":0});
    assert_eq!(
        tools.authorization("read_process", &args).unwrap().risk(),
        ToolRisk::ReadOnly
    );
    let result = run(&tools, "read_process", args).unwrap();
    assert_eq!(result["stdout"], "ok");
    assert_eq!(result["stderr_view"]["total_lines"], 0);
    assert!(result["stderr_view"]["start_line"].is_null());
}

#[test]
fn configured_cli_sessions_keep_request_local_background_rejection() {
    use atlas_agentic::{process_limits::ProcessLimits, tools::ExecCommandTool};
    let root = tempfile::tempdir().unwrap();
    let exec = ExecCommandTool::new(root.path())
        .with_process_limits(ProcessLimits {
            per_chat: 1,
            total: 2,
        })
        .unwrap();
    let mut registry = ToolRegistry::new(exec.session_tools());
    registry.register(Box::new(exec));
    let first = run(
        &registry,
        "exec_command",
        json!({"command":"sleep 20","yield_time_ms":0}),
    )
    .unwrap();
    assert!(run(
        &registry,
        "exec_command",
        json!({"command":"touch rejected","yield_time_ms":0})
    )
    .is_err());
    let background = run(
        &registry,
        "exec_command",
        json!({"command":"sleep 20","lifetime":"background","yield_time_ms":0}),
    )
    .unwrap_err();
    assert!(
        background.to_lowercase().contains("background"),
        "{background}"
    );
    assert!(!root.path().join("rejected").exists());
    run(
        &registry,
        "stop_process",
        json!({"session_id":first["session_id"]}),
    )
    .unwrap();
}
