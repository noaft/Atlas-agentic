use atlas_agentic::{executions::ToolExecutionManager, tools::ToolResult};
use std::{thread, time::Duration};

#[test]
fn completion_wakes_early_without_exposing_output_or_replaying_work() {
    let manager = ToolExecutionManager::new();
    let owner = manager.scope("owner");
    let other = manager.scope("other");
    let info = owner
        .start("fake", move |reporter| {
            thread::sleep(Duration::from_millis(25));
            reporter.append("private log\n");
            Ok(ToolResult::text("finished"))
        })
        .unwrap();
    assert!(other.read(&info.execution_id, Some(20), None).is_err());
    let started = std::time::Instant::now();
    let batch = owner
        .wait(std::slice::from_ref(&info.execution_id), 2000)
        .unwrap();
    assert!(started.elapsed() < Duration::from_secs(1));
    let json = serde_json::to_string(&batch).unwrap();
    assert!(!json.contains("private log"));
    assert!(!json.contains("finished"));
    assert_eq!(batch.events.len(), 1);
    assert_eq!(batch.events[0].status, "succeeded");
    owner.ack(&batch.batch_id).unwrap();
    assert!(owner.claim().unwrap().is_none());
}

#[test]
fn line_selection_is_explicit_and_capped_even_for_unicode_single_lines() {
    let manager = ToolExecutionManager::new();
    let scope = manager.scope("owner");
    let info = scope
        .start("fake", |_| {
            Ok(ToolResult::text(format!("{}\nlast\n", "ế".repeat(5000))))
        })
        .unwrap();
    let batch = scope
        .wait(std::slice::from_ref(&info.execution_id), 100)
        .unwrap();
    assert_eq!(batch.events[0].status, "succeeded");
    let tail = scope.read(&info.execution_id, Some(1), None).unwrap();
    assert_eq!(tail.text, "last\n");
    let all = scope.read(&info.execution_id, Some(200), None).unwrap();
    assert!(all.text.len() <= 4096);
    assert!(all.truncated);
    assert!(scope.read(&info.execution_id, Some(201), None).is_err());
    assert!(scope.read(&info.execution_id, Some(1), Some(0)).is_err());
    assert!(scope.wait(&[info.execution_id], 30000).is_err());
}

#[test]
fn in_flight_delivery_cannot_erase_a_later_event_and_consumers_are_exclusive() {
    let manager = ToolExecutionManager::new();
    let scope = manager.scope("owner");
    let _lease = scope.acquire_consumer().unwrap();
    assert!(scope.acquire_consumer().is_err());
    let (tx, rx) = std::sync::mpsc::channel();
    let info = scope
        .start("fake", move |_| {
            rx.recv().unwrap();
            Ok(ToolResult::text("done"))
        })
        .unwrap();
    let periodic = scope
        .wait(std::slice::from_ref(&info.execution_id), 100)
        .unwrap();
    assert_eq!(periodic.events[0].reason, "interval_elapsed");
    tx.send(()).unwrap();
    for _ in 0..100 {
        if scope.info(&info.execution_id).unwrap().status == "succeeded" {
            break;
        }
        thread::sleep(Duration::from_millis(2));
    }
    scope.ack(&periodic.batch_id).unwrap();
    let completed = scope.claim().unwrap().unwrap();
    assert_eq!(completed.events[0].reason, "completed");
    scope.ack(&completed.batch_id).unwrap();
    assert!(scope.claim().unwrap().is_none());
}

#[test]
fn wakeup_protocol_rejects_long_sleeps_and_unbounded_reads() {
    use atlas_agentic::agent_output::AgentOutput;
    assert!(AgentOutput::parse_tool_protocol(
        r#"{"type":"tool_call","tool":"pwd","execution":{"mode":"job","yield_ms":30000},"args":{}}"#
    )
    .is_err());
    assert!(AgentOutput::parse_tool_protocol(
        r#"{"type":"read_execution","execution_id":"x","tail_lines":201}"#
    )
    .is_err());
    assert!(AgentOutput::parse_tool_protocol(
        r#"{"type":"await_execution","execution_id":"x","yield_ms":2000}"#
    )
    .is_ok());
}

mod runtime_wakeup {
    use super::*;
    use atlas_agentic::{
        agents::{AgentRegistry, LocalAgent, LocalAgentDefinition},
        models::{ModelProvider, ModelRequest},
        permissions::{AllowAllAuthorizer, PermissionMode},
        tools::{Tool, ToolDefinition, ToolRegistry},
        Runtime,
    };
    use std::{
        cell::Cell,
        sync::{
            atomic::{AtomicBool, AtomicUsize, Ordering},
            Arc,
        },
    };
    struct Slow {
        finish: Arc<AtomicBool>,
        calls: Arc<AtomicUsize>,
    }
    impl Tool for Slow {
        fn name(&self) -> &str {
            "slow"
        }
        fn definition(&self) -> ToolDefinition {
            ToolDefinition {
                name: "slow".into(),
                description: "Fake slow tool".into(),
                input_schema: serde_json::json!({"type":"object"}),
            }
        }
        fn execute(&self, _: &serde_json::Value) -> Result<ToolResult, String> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            let deadline = std::time::Instant::now() + Duration::from_secs(3);
            while !self.finish.load(Ordering::SeqCst) && std::time::Instant::now() < deadline {
                thread::sleep(Duration::from_millis(2));
            }
            Ok(ToolResult::text(format!(
                "RAW_LOG_SHOULD_NOT_PRELOAD\n{}\nlast one\nlast two\n",
                "x".repeat(20000)
            )))
        }
    }
    struct Model {
        step: Cell<usize>,
        finish: Arc<AtomicBool>,
    }
    impl ModelProvider for Model {
        fn complete(&self, request: &ModelRequest) -> Result<String, String> {
            let step = self.step.get();
            self.step.set(step + 1);
            if step == 0 {
                return Ok(r#"{"type":"tool_call","tool":"slow","execution":{"mode":"job","yield_ms":100},"args":{}}"#.into());
            }
            let input: serde_json::Value = serde_json::from_str(request.user_prompt).unwrap();
            let id = input["executions"][0]["execution_id"].as_str().unwrap();
            match step {
                1 => {
                    assert_eq!(
                        input["execution_wakeup"]["events"][0]["reason"],
                        "interval_elapsed"
                    );
                    assert!(!request.user_prompt.contains("RAW_LOG"));
                    self.finish.store(true, Ordering::SeqCst);
                    Ok(format!(
                        r#"{{"type":"await_execution","execution_id":"{id}","yield_ms":1500}}"#
                    ))
                }
                2 => {
                    assert_eq!(
                        input["execution_wakeup"]["events"][0]["status"],
                        "succeeded"
                    );
                    assert!(!request.user_prompt.contains("RAW_LOG"));
                    Ok(format!(
                        r#"{{"type":"read_execution","execution_id":"{id}","tail_lines":2}}"#
                    ))
                }
                3 => {
                    let record = input["tool_results"].as_array().unwrap().last().unwrap();
                    let read: serde_json::Value =
                        serde_json::from_str(record["output"].as_str().unwrap()).unwrap();
                    assert_eq!(read["result"]["text"], "last one\nlast two\n");
                    assert!(!request.user_prompt.contains("RAW_LOG"));
                    Ok(r#"{"type":"final","content":"verified"}"#.into())
                }
                _ => panic!("Unexpected repeat / extra model call"),
            }
        }
    }
    #[test]
    fn runtime_changes_its_sleep_reads_only_selected_lines_and_runs_once() {
        let finish = Arc::new(AtomicBool::new(false));
        let calls = Arc::new(AtomicUsize::new(0));
        let definition=LocalAgentDefinition::from_toml("name='atlas'\ndisplay_name='Atlas'\nmodel_provider='fake'\nmodel='fake'\nsystem_prompt='Test'\nconversation_title_prompt='Title'").unwrap();
        let runtime = Runtime::new(AgentRegistry::new(vec![Box::new(LocalAgent::new(
            definition,
            Box::new(Model {
                step: Cell::new(0),
                finish: finish.clone(),
            }),
        ))]))
        .with_tools(ToolRegistry::new(vec![
            atlas_agentic::executions::worker::tool(Slow {
                finish,
                calls: calls.clone(),
            }),
        ]));
        let answer = runtime
            .ask_stream_with_history_and_authorization(
                "run",
                &[],
                PermissionMode::FullAccess,
                &AllowAllAuthorizer,
                &mut |_| {},
                &mut |_| {},
            )
            .unwrap();
        assert_eq!(answer, "verified");
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }
    struct EmptyControls(Cell<usize>);
    impl ModelProvider for EmptyControls {
        fn complete(&self, request: &ModelRequest) -> Result<String, String> {
            let step = self.0.get();
            self.0.set(step + 1);
            if step == 0 {
                return Ok(r#"{"type":"list_executions"}"#.into());
            }
            let value: serde_json::Value = serde_json::from_str(request.user_prompt)
                .expect("control result must reach the model even when empty");
            let last = value["tool_results"].as_array().unwrap().last().unwrap();
            let output: serde_json::Value =
                serde_json::from_str(last["output"].as_str().unwrap()).unwrap();
            if step == 1 {
                assert_eq!(output["result"], serde_json::json!([]));
                return Ok(
                    r#"{"type":"read_execution","execution_id":"foreign","tail_lines":2}"#.into(),
                );
            }
            assert_eq!(output["success"], false);
            assert!(output["error"].as_str().unwrap().contains("not found"));
            Ok(r#"{"type":"final","content":"empty scope handled"}"#.into())
        }
    }
    #[test]
    fn empty_control_results_and_foreign_id_errors_are_delivered() {
        let definition=LocalAgentDefinition::from_toml("name='atlas'\ndisplay_name='Atlas'\nmodel_provider='fake'\nmodel='fake'\nsystem_prompt='Test'\nconversation_title_prompt='Title'").unwrap();
        let runtime = Runtime::new(AgentRegistry::new(vec![Box::new(LocalAgent::new(
            definition,
            Box::new(EmptyControls(Cell::new(0))),
        ))]));
        assert_eq!(
            runtime.ask_stream("inspect", &mut |_| {}).unwrap(),
            "empty scope handled"
        );
    }
    struct FailureAfterWake {
        started: Cell<bool>,
        batch: Arc<std::sync::Mutex<String>>,
    }
    impl ModelProvider for FailureAfterWake {
        fn complete(&self, request: &ModelRequest) -> Result<String, String> {
            if !self.started.replace(true) {
                return Ok(r#"{"type":"tool_call","tool":"slow","execution":{"mode":"job","yield_ms":100},"args":{}}"#.into());
            }
            let value: serde_json::Value = serde_json::from_str(request.user_prompt).unwrap();
            *self.batch.lock().unwrap() = value["execution_wakeup"]["batch_id"]
                .as_str()
                .unwrap()
                .into();
            Err("simulated transport failure".into())
        }
    }
    #[test]
    fn provider_failure_preserves_delivery_requests_job_cancel_and_never_replays() {
        let finish = Arc::new(AtomicBool::new(false));
        let calls = Arc::new(AtomicUsize::new(0));
        let batch = Arc::new(std::sync::Mutex::new(String::new()));
        let manager = ToolExecutionManager::new();
        let scope = manager.scope("owner");
        let definition=LocalAgentDefinition::from_toml("name='atlas'\ndisplay_name='Atlas'\nmodel_provider='fake'\nmodel='fake'\nsystem_prompt='Test'\nconversation_title_prompt='Title'").unwrap();
        let runtime = Runtime::new(AgentRegistry::new(vec![Box::new(LocalAgent::new(
            definition,
            Box::new(FailureAfterWake {
                started: Cell::new(false),
                batch: batch.clone(),
            }),
        ))]))
        .with_tools(ToolRegistry::new(vec![
            atlas_agentic::executions::worker::tool(Slow {
                finish: finish.clone(),
                calls: calls.clone(),
            }),
        ]))
        .with_execution_scope(scope.clone());
        assert!(runtime.ask_stream("run", &mut |_| {}).is_err());
        assert!(scope.list().unwrap()[0].cancel_requested);
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        let old = scope.claim().unwrap().unwrap();
        assert_eq!(old.batch_id, *batch.lock().unwrap());
        let _recovered_consumer = scope.acquire_consumer().unwrap();
        scope.ack(&old.batch_id).unwrap();
        finish.store(true, Ordering::SeqCst);
        let id = scope.list().unwrap()[0].execution_id.clone();
        let completed = scope.wait(&[id], 1000).unwrap();
        assert_eq!(completed.events[0].status, "succeeded");
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }
}

#[test]
fn running_output_is_never_a_complete_result() {
    let manager = ToolExecutionManager::new();
    let scope = manager.scope("owner");
    let (release, gate) = std::sync::mpsc::channel();
    let info = scope
        .start("fake", move |r| {
            r.append("partial\n");
            gate.recv().unwrap();
            Ok(ToolResult::text("done"))
        })
        .unwrap();
    let batch = scope
        .wait(std::slice::from_ref(&info.execution_id), 100)
        .unwrap();
    assert!(
        !scope
            .read(&info.execution_id, Some(20), None)
            .unwrap()
            .complete_result
    );
    scope.ack(&batch.batch_id).unwrap();
    release.send(()).unwrap();
}

#[cfg(unix)]
#[test]
fn background_wakeup_tracks_real_exit_and_signal_failure() {
    use atlas_agentic::tools::{ProcessManager, ToolRegistry};
    let root = tempfile::tempdir().unwrap();
    let processes = ProcessManager::new();
    let process_scope = processes.create_scope().unwrap();
    let mut tools = ToolRegistry::empty();
    atlas_agentic::tools::register_process_tools_in_scope(&mut tools, root.path(), process_scope);
    let manager = ToolExecutionManager::new();
    let scope = manager.scope("owner");
    for (command, background) in [
        (
            "while [ ! -f release ]; do sleep .01; done; printf done; exit 7",
            true,
        ),
        ("kill -TERM $$", false),
    ] {
        let args = serde_json::json!({"command":command,"yield_time_ms":0,"timeout_ms":5000,"lifetime":if background{"background"}else{"job"}});
        let authorization = tools.authorization("exec_command", &args).unwrap();
        let work = tools.prepare_execution(authorization).unwrap();
        let info = scope.start("exec_command", work).unwrap();
        if background {
            let batch = scope
                .wait(std::slice::from_ref(&info.execution_id), 100)
                .unwrap();
            assert_eq!(batch.events[0].status, "running");
            scope.ack(&batch.batch_id).unwrap();
            std::fs::write(root.path().join("release"), "ready").unwrap();
        }
        loop {
            let batch = scope
                .wait(std::slice::from_ref(&info.execution_id), 500)
                .unwrap();
            let status = batch.events[0].status.clone();
            scope.ack(&batch.batch_id).unwrap();
            if status != "running" {
                assert_eq!(status, "failed");
                break;
            }
        }
    }
    processes.shutdown(Duration::from_secs(2)).unwrap();
}

#[test]
fn error_details_obey_the_same_line_selection_as_logs() {
    let manager = ToolExecutionManager::new();
    let scope = manager.scope("owner");
    let info = scope
        .start("fake", |_| {
            Err((0..100).map(|i| format!("error line {i}\n")).collect())
        })
        .unwrap();
    let batch = scope
        .wait(std::slice::from_ref(&info.execution_id), 100)
        .unwrap();
    assert_eq!(batch.events[0].status, "failed");
    let read = scope.read(&info.execution_id, Some(1), None).unwrap();
    assert_eq!(read.text, "error line 99\n");
    assert!(!read.error.unwrap().contains("error line 0"));
}

#[test]
fn generic_execution_caps_reject_before_the_extra_worker_runs() {
    use std::sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc,
    };
    let manager = ToolExecutionManager::new();
    let release = Arc::new(AtomicBool::new(false));
    let calls = Arc::new(AtomicUsize::new(0));
    let (ready, started) = std::sync::mpsc::channel();
    for n in 0..4 {
        let scope = manager.scope(&format!("owner-{n}"));
        for _ in 0..8 {
            let release = release.clone();
            let calls = calls.clone();
            let ready = ready.clone();
            scope
                .start("fake", move |r| {
                    calls.fetch_add(1, Ordering::SeqCst);
                    ready.send(()).unwrap();
                    while !release.load(Ordering::SeqCst) && !r.cancelled() {
                        thread::sleep(Duration::from_millis(2));
                    }
                    Ok(ToolResult::text("done"))
                })
                .unwrap();
        }
        let calls = calls.clone();
        assert!(scope
            .start("extra", move |_| {
                calls.fetch_add(1, Ordering::SeqCst);
                Ok(ToolResult::text("unexpected"))
            })
            .is_err());
    }
    for _ in 0..32 {
        started.recv_timeout(Duration::from_secs(1)).unwrap();
    }
    assert!(manager
        .scope("new-owner")
        .start("extra", |_| Ok(ToolResult::text("unexpected")))
        .is_err());
    assert_eq!(calls.load(Ordering::SeqCst), 32);
    release.store(true, Ordering::SeqCst);
    manager.shutdown(Duration::from_secs(2)).unwrap();
}

#[test]
fn worker_keeps_the_exact_destructive_authorization_snapshot() {
    use atlas_agentic::tools::{ApplyPatchTool, ToolRegistry};
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("file.txt");
    std::fs::write(&path, "before").unwrap();
    let tools = ToolRegistry::new(vec![atlas_agentic::executions::worker::tool(
        ApplyPatchTool::new(root.path()),
    )]);
    let args =
        serde_json::json!({"path":"file.txt","edits":[{"old_text":"before","new_text":"patched"}]});
    let authorization = tools.authorization("apply_patch", &args).unwrap();
    assert_eq!(
        authorization.risk(),
        atlas_agentic::permissions::ToolRisk::Destructive
    );
    std::fs::write(&path, "newer contents").unwrap();
    let work = tools.prepare_execution(authorization).unwrap();
    let manager = ToolExecutionManager::new();
    let scope = manager.scope("owner");
    let info = scope.start("apply_patch", work).unwrap();
    let batch = scope
        .wait(std::slice::from_ref(&info.execution_id), 100)
        .unwrap();
    assert_eq!(batch.events[0].status, "failed");
    assert_eq!(std::fs::read_to_string(path).unwrap(), "newer contents");
}
