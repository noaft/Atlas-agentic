#![cfg(unix)]
use atlas_agentic::{
    agent_output::AgentOutput,
    tools::{register_process_tools_in_scope, ProcessCursor, ProcessManager, ToolRegistry},
};
use serde_json::{json, Value};
use std::{path::Path, time::Duration};
fn tools(path: &Path, scope: atlas_agentic::tools::ProcessScope) -> ToolRegistry {
    let mut registry = ToolRegistry::empty();
    register_process_tools_in_scope(&mut registry, path, scope);
    registry
}
fn call(registry: &ToolRegistry, name: &str, args: Value) -> Result<Value, String> {
    let auth = registry.authorization(name, &args)?;
    let result = registry.execute_authorized(
        &AgentOutput::ToolCall {
            tool: name.into(),
            args,
            message: None,
        },
        &auth,
    )?;
    serde_json::from_str(&result.message).map_err(|e| e.to_string())
}
#[test]
fn background_survives_runtime_drop_and_stays_owned_by_one_chat() {
    let root = tempfile::tempdir().unwrap();
    let manager = ProcessManager::new();
    let chat = manager.create_scope().unwrap();
    let other = manager.create_scope().unwrap();
    let first = tools(root.path(), chat.clone());
    let result = call(
        &first,
        "exec_command",
        json!({"command":"printf ready; sleep 10","lifetime":"background","yield_time_ms":50}),
    )
    .unwrap();
    let id = result["session_id"].as_str().unwrap();
    drop(first);
    assert_eq!(chat.list().unwrap().len(), 1);
    assert!(other.read(id, Some(ProcessCursor::default()), 0).is_err());
    assert!(other.stop(id).is_err());
    let second = tools(root.path(), chat.clone());
    let listed = call(&second, "list_processes", json!({})).unwrap();
    assert_eq!(listed["processes"][0]["sessionId"], id);
    let stopped = call(&second, "stop_process", json!({"session_id":id})).unwrap();
    assert_eq!(stopped["running"], false);
    assert_eq!(stopped["stopped"], true);
}
#[test]
fn ui_cursor_does_not_consume_agent_output_and_job_is_cleaned_on_drop() {
    let root = tempfile::tempdir().unwrap();
    let manager = ProcessManager::new();
    let chat = manager.create_scope().unwrap();
    let registry = tools(root.path(), chat.clone());
    let first = call(
        &registry,
        "exec_command",
        json!({"command":"sleep .1; printf 'Tiếng Việt'; sleep 10","yield_time_ms":0}),
    )
    .unwrap();
    let id = first["session_id"].as_str().unwrap();
    let viewer = chat.read(id, Some(ProcessCursor::default()), 1000).unwrap();
    assert_eq!(viewer.stdout, "Tiếng Việt");
    let agent = call(
        &registry,
        "read_process",
        json!({"session_id":id,"wait_ms":0}),
    )
    .unwrap();
    assert_eq!(agent["stdout"], "Tiếng Việt");
    let again = chat.read(id, Some(viewer.next_cursor), 0).unwrap();
    assert_eq!(again.stdout, "");
    drop(registry);
    assert!(!chat.list().unwrap()[0].running);
    chat.close(Duration::from_secs(2)).unwrap();
    assert!(chat.list().is_err());
}
#[test]
fn global_active_limit_is_checked_across_chat_scopes_before_spawn() {
    let root = tempfile::tempdir().unwrap();
    let manager = ProcessManager::new();
    let mut owners = Vec::new();
    for _ in 0..4 {
        let scope = manager.create_scope().unwrap();
        let registry = tools(root.path(), scope.clone());
        for _ in 0..8 {
            call(
                &registry,
                "exec_command",
                json!({"command":"sleep 20","lifetime":"background","yield_time_ms":0}),
            )
            .unwrap();
        }
        owners.push((scope, registry));
    }
    let scope = manager.create_scope().unwrap();
    let registry = tools(root.path(), scope);
    assert!(call(
        &registry,
        "exec_command",
        json!({"command":"touch forbidden","lifetime":"background","yield_time_ms":0})
    )
    .is_err());
    assert!(!root.path().join("forbidden").exists());
    manager.shutdown(Duration::from_secs(3)).unwrap();
}

#[test]
fn scoped_synchronous_commands_are_visible_and_count_toward_manager_limits() {
    let root = tempfile::tempdir().unwrap();
    let manager = ProcessManager::new();
    let chat = manager.create_scope().unwrap();
    let path = root.path().to_path_buf();
    let owner = chat.clone();
    let worker = std::thread::spawn(move || {
        let registry = tools(&path, owner);
        call(
            &registry,
            "exec_command",
            json!({"command":"sleep .2; printf sync"}),
        )
        .unwrap()
    });
    let deadline = std::time::Instant::now() + Duration::from_secs(2);
    while chat.list().unwrap().is_empty() && std::time::Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(chat.list().unwrap().len(), 1);
    let result = worker.join().unwrap();
    assert_eq!(result["stdout"], "sync");
    assert_eq!(result["exit_code"], 0);
}

#[test]
fn ownership_transfer_preserves_ids_and_revokes_the_old_scope() {
    let root = tempfile::tempdir().unwrap();
    let manager = ProcessManager::new();
    let source = manager.create_scope().unwrap();
    let target = manager.create_scope().unwrap();
    let registry = tools(root.path(), source.clone());
    let result = call(
        &registry,
        "exec_command",
        json!({"command":"printf preserved; sleep 20","lifetime":"background","yield_time_ms":50}),
    )
    .unwrap();
    let id = result["session_id"].as_str().unwrap();
    drop(registry);
    target.adopt_from(&source).unwrap();
    assert!(source.list().is_err());
    assert_eq!(target.list().unwrap()[0].session_id, id);
    assert_eq!(
        target
            .read(id, Some(ProcessCursor::default()), 0)
            .unwrap()
            .stdout,
        "preserved"
    );
    target.stop(id).unwrap();
}

#[test]
fn live_catalog_keeps_old_running_background_ahead_of_recent_completed_jobs() {
    let root = tempfile::tempdir().unwrap();
    let manager = ProcessManager::new();
    let scope = manager.create_scope().unwrap();
    let registry = tools(root.path(), scope.clone());
    let result = call(
        &registry,
        "exec_command",
        json!({"command":"sleep 20","lifetime":"background","yield_time_ms":0}),
    )
    .unwrap();
    let id = result["session_id"].as_str().unwrap();
    for _ in 0..15 {
        call(&registry, "exec_command", json!({"command":"true"})).unwrap();
    }
    let descriptor = registry
        .definitions()
        .into_iter()
        .find(|d| d.name == "list_processes")
        .unwrap();
    assert!(descriptor.description.contains(id));
    scope.stop(id).unwrap();
}
#[test]
fn ui_stop_does_not_advance_the_agent_reader() {
    let root = tempfile::tempdir().unwrap();
    let manager = ProcessManager::new();
    let scope = manager.create_scope().unwrap();
    let registry = tools(root.path(), scope.clone());
    let started=call(&registry,"exec_command",json!({"command":"sleep .1; printf unread; sleep 20","lifetime":"background","yield_time_ms":0})).unwrap();
    let id = started["session_id"].as_str().unwrap();
    assert_eq!(
        scope
            .read(id, Some(ProcessCursor::default()), 1000)
            .unwrap()
            .stdout,
        "unread"
    );
    scope.stop(id).unwrap();
    assert_eq!(
        call(
            &registry,
            "read_process",
            json!({"session_id":id,"wait_ms":0})
        )
        .unwrap()["stdout"],
        "unread"
    );
}

#[test]
fn completed_records_are_bounded_per_chat_and_across_the_backend() {
    let root = tempfile::tempdir().unwrap();
    let manager = ProcessManager::new();
    let mut scopes = Vec::new();
    for _ in 0..4 {
        let scope = manager.create_scope().unwrap();
        let registry = tools(root.path(), scope.clone());
        for _ in 0..33 {
            call(&registry, "exec_command", json!({"command":"true"})).unwrap();
        }
        drop(registry);
        assert_eq!(scope.list().unwrap().len(), 32);
        scopes.push(scope);
    }
    assert_eq!(
        scopes
            .iter()
            .map(|scope| scope.list().unwrap().len())
            .sum::<usize>(),
        128
    );
    let extra = manager.create_scope().unwrap();
    let registry = tools(root.path(), extra.clone());
    call(&registry, "exec_command", json!({"command":"true"})).unwrap();
    scopes.push(extra);
    assert_eq!(
        scopes
            .iter()
            .map(|scope| scope.list().unwrap().len())
            .sum::<usize>(),
        128
    );
}

#[test]
fn configurable_limits_enforce_each_chat_and_the_app_before_launch() {
    use atlas_agentic::process_limits::ProcessLimits;
    let root = tempfile::tempdir().unwrap();
    let manager = ProcessManager::new();
    manager
        .set_limits(ProcessLimits {
            per_chat: 1,
            total: 2,
        })
        .unwrap();
    let a = manager.create_scope().unwrap();
    let b = manager.create_scope().unwrap();
    let c = manager.create_scope().unwrap();
    let ta = tools(root.path(), a.clone());
    let tb = tools(root.path(), b);
    let tc = tools(root.path(), c);
    let start = json!({"command":"sleep 20","lifetime":"background","yield_time_ms":0});
    call(&ta, "exec_command", start.clone()).unwrap();
    let denied =
        json!({"command":"touch should-not-start","lifetime":"background","yield_time_ms":0});
    assert!(call(&ta, "exec_command", denied.clone()).is_err());
    call(&tb, "exec_command", start.clone()).unwrap();
    assert!(call(&tc, "exec_command", denied).is_err());
    assert!(!root.path().join("should-not-start").exists());
    a.close(Duration::from_secs(2)).unwrap();
    call(&tc, "exec_command", start).unwrap();
    manager.shutdown(Duration::from_secs(2)).unwrap();
}

#[test]
fn lowering_limits_keeps_existing_commands_and_blocks_until_they_drain() {
    use atlas_agentic::process_limits::ProcessLimits;
    let root = tempfile::tempdir().unwrap();
    let manager = ProcessManager::new();
    manager
        .set_limits(ProcessLimits {
            per_chat: 2,
            total: 2,
        })
        .unwrap();
    let chat = manager.create_scope().unwrap();
    let registry = tools(root.path(), chat.clone());
    let start = json!({"command":"sleep 20","lifetime":"background","yield_time_ms":0});
    let first = call(&registry, "exec_command", start.clone()).unwrap();
    let second = call(&registry, "exec_command", start.clone()).unwrap();
    manager
        .set_limits(ProcessLimits {
            per_chat: 1,
            total: 1,
        })
        .unwrap();
    assert!(chat.list().unwrap().iter().all(|entry| entry.running));
    assert!(call(&registry, "exec_command", start.clone()).is_err());
    chat.stop(first["session_id"].as_str().unwrap()).unwrap();
    assert!(call(&registry, "exec_command", start.clone()).is_err());
    chat.stop(second["session_id"].as_str().unwrap()).unwrap();
    call(&registry, "exec_command", start.clone()).unwrap();
    manager
        .set_limits(ProcessLimits {
            per_chat: 2,
            total: 2,
        })
        .unwrap();
    call(&registry, "exec_command", start).unwrap();
    manager.shutdown(Duration::from_secs(2)).unwrap();
}

#[test]
fn concurrent_admission_cannot_bypass_a_single_command_limit() {
    use atlas_agentic::process_limits::ProcessLimits;
    use std::sync::{Arc, Barrier};
    let root = tempfile::tempdir().unwrap();
    let manager = ProcessManager::new();
    manager
        .set_limits(ProcessLimits {
            per_chat: 1,
            total: 1,
        })
        .unwrap();
    let scope = manager.create_scope().unwrap();
    let barrier = Arc::new(Barrier::new(3));
    let mut workers = Vec::new();
    for _ in 0..2 {
        let path = root.path().to_path_buf();
        let chat = scope.clone();
        let gate = barrier.clone();
        workers.push(std::thread::spawn(move || {
            let registry = tools(&path, chat);
            gate.wait();
            call(
                &registry,
                "exec_command",
                json!({"command":"sleep 20","lifetime":"background","yield_time_ms":0}),
            )
            .is_ok()
        }));
    }
    barrier.wait();
    let admitted = workers
        .into_iter()
        .map(|worker| usize::from(worker.join().unwrap()))
        .sum::<usize>();
    assert_eq!(admitted, 1);
    manager.shutdown(Duration::from_secs(2)).unwrap();
}

#[test]
fn draft_handoff_remains_valid_after_lowering_below_existing_activity() {
    use atlas_agentic::process_limits::ProcessLimits;
    let root = tempfile::tempdir().unwrap();
    let manager = ProcessManager::new();
    manager
        .set_limits(ProcessLimits {
            per_chat: 2,
            total: 2,
        })
        .unwrap();
    let draft = manager.create_scope().unwrap();
    let saved = manager.create_scope().unwrap();
    let registry = tools(root.path(), draft.clone());
    let start = json!({"command":"sleep 20","lifetime":"background","yield_time_ms":0});
    call(&registry, "exec_command", start.clone()).unwrap();
    call(&registry, "exec_command", start.clone()).unwrap();
    manager
        .set_limits(ProcessLimits {
            per_chat: 1,
            total: 1,
        })
        .unwrap();
    saved.adopt_from(&draft).unwrap();
    assert_eq!(saved.list().unwrap().len(), 2);
    assert!(saved.list().unwrap().iter().all(|entry| entry.running));
    assert!(call(&tools(root.path(), saved), "exec_command", start).is_err());
    manager.shutdown(Duration::from_secs(2)).unwrap();
}
