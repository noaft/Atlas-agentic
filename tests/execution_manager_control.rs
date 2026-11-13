fn snapshot_path(root: &std::path::Path) -> std::path::PathBuf {
    std::fs::read_dir(root)
        .unwrap()
        .filter_map(Result::ok)
        .filter(|e| uuid::Uuid::parse_str(&e.file_name().to_string_lossy()).is_ok())
        .map(|e| e.path().join("executions.json"))
        .next()
        .unwrap()
}
use atlas_agentic::executions::{
    protocol::{ExecutionPolicy, WakeupMode},
    ToolExecutionManager,
};
use atlas_agentic::tools::ToolResult;
#[test]
fn lazy_policy_survives_explicit_wait_and_completion_is_immediate() {
    let manager = ToolExecutionManager::new();
    let scope = manager.scope("chat");
    let (send, recv) = std::sync::mpsc::channel();
    let run = scope
        .start("test", move |_| {
            recv.recv().unwrap();
            Ok(ToolResult::text("done"))
        })
        .unwrap();
    let policy = ExecutionPolicy {
        mode: WakeupMode::Lazy,
        interval_seconds: None,
    };
    scope.set_wakeup(&run.execution_id, policy.clone()).unwrap();
    scope
        .wait(std::slice::from_ref(&run.execution_id), 100)
        .unwrap();
    assert_eq!(scope.info(&run.execution_id).unwrap().wake_up, policy);
    send.send(()).unwrap();
    let batch = scope
        .wait_scheduled_checked(std::slice::from_ref(&run.execution_id), || Ok(()))
        .unwrap();
    // The previous interval notification remains in flight until acknowledged.
    scope.ack(&batch.batch_id).unwrap();
    let batch = scope
        .wait_scheduled_checked(&[run.execution_id], || Ok(()))
        .unwrap();
    assert_eq!(batch.events[0].status, "succeeded");
}
#[test]
fn durable_results_survive_restart_without_replaying_tools() {
    let dir = tempfile::tempdir().unwrap();
    let id;
    {
        let manager = ToolExecutionManager::with_persistence(dir.path()).unwrap();
        let scope = manager.scope("saved-chat");
        id = scope
            .start("test", |_| Ok(ToolResult::text("durable result")))
            .unwrap()
            .execution_id;
        let batch = scope.wait(std::slice::from_ref(&id), 100).unwrap();
        scope.ack(&batch.batch_id).unwrap();
    }
    let manager = ToolExecutionManager::with_persistence(dir.path()).unwrap();
    let scope = manager.scope("saved-chat");
    assert_eq!(scope.info(&id).unwrap().status, "succeeded");
    assert_eq!(
        scope.read(&id, None, Some(0)).unwrap().text,
        "durable result"
    );
    assert!(manager.scope("other").info(&id).is_err());
    assert!(scope.claim().unwrap().is_none());
}
#[test]
fn checked_owner_removal_trashes_results_and_rejects_late_publication() {
    let dir = tempfile::tempdir().unwrap();
    let manager = ToolExecutionManager::with_persistence(dir.path()).unwrap();
    manager.bind_persistence_owner("native", "saved").unwrap();
    let scope = manager.scope("native");
    let (send, recv) = std::sync::mpsc::channel();
    let (ready, started) = std::sync::mpsc::channel();
    let run = scope
        .start("test", move |_| {
            ready.send(()).unwrap();
            recv.recv().unwrap();
            Ok(ToolResult::text("late"))
        })
        .unwrap();
    started.recv().unwrap();
    manager.remove_persistence_owner("saved").unwrap();
    send.send(()).unwrap();
    assert!(scope.info(&run.execution_id).is_err());
    let persisted = std::fs::read_to_string(snapshot_path(dir.path())).unwrap();
    assert!(!persisted.contains(&run.execution_id));
    assert_eq!(
        std::fs::read_dir(dir.path().join("trash")).unwrap().count(),
        1
    );
}
#[test]
fn durable_record_quota_blocks_dispatch_when_unacknowledged() {
    use atlas_agentic::executions::PersistenceLimits;
    let dir = tempfile::tempdir().unwrap();
    let manager = ToolExecutionManager::with_persistence_options(
        dir.path(),
        PersistenceLimits {
            max_records: 1,
            max_bytes: 1024 * 1024,
        },
    )
    .unwrap();
    let scope = manager.scope("saved");
    let run = scope
        .start("test", |_| Ok(ToolResult::text("first")))
        .unwrap();
    let batch = scope.wait(&[run.execution_id], 100).unwrap();
    assert!(scope
        .start("test", |_| panic!("must not dispatch"))
        .is_err());
    scope.ack(&batch.batch_id).unwrap();
    scope
        .start("test", |_| Ok(ToolResult::text("second")))
        .unwrap();
}
#[cfg(unix)]
#[test]
fn durable_snapshots_are_private_and_reject_symlink_targets() {
    use std::os::unix::{fs::symlink, fs::PermissionsExt};
    let dir = tempfile::tempdir().unwrap();
    let manager = ToolExecutionManager::with_persistence(dir.path()).unwrap();
    let file = snapshot_path(dir.path());
    assert_eq!(
        std::fs::metadata(&file).unwrap().permissions().mode() & 0o777,
        0o600
    );
    std::fs::remove_file(&file).unwrap();
    let outside = dir.path().join("outside");
    std::fs::write(&outside, "untouched").unwrap();
    symlink(&outside, &file).unwrap();
    assert!(manager
        .scope("owner")
        .start("test", |_| panic!("must not dispatch"))
        .is_err());
    assert_eq!(std::fs::read_to_string(outside).unwrap(), "untouched");
}
#[test]
fn durable_large_utf8_results_remain_bounded_after_restart() {
    let dir = tempfile::tempdir().unwrap();
    let id;
    {
        let manager = ToolExecutionManager::with_persistence(dir.path()).unwrap();
        let scope = manager.scope("owner");
        id = scope
            .start("test", |_| Ok(ToolResult::text("界\n".repeat(40000))))
            .unwrap()
            .execution_id;
        let batch = scope.wait(std::slice::from_ref(&id), 100).unwrap();
        scope.ack(&batch.batch_id).unwrap();
    }
    let manager = ToolExecutionManager::with_persistence(dir.path()).unwrap();
    let scope = manager.scope("owner");
    let read = scope.read(&id, Some(200), None).unwrap();
    assert!(read.retention_loss);
    assert!(read.text.len() <= 4096);
    assert_eq!(read.end_cursor, 160000);
    assert!(read.oldest_cursor >= 160000 - 65536);
}
#[test]
fn execution_identity_includes_request_and_durable_timestamps() {
    let dir = tempfile::tempdir().unwrap();
    let manager = ToolExecutionManager::with_persistence(dir.path()).unwrap();
    let scope = manager.scope("owner");
    let _consumer = scope.acquire_consumer().unwrap();
    let first = scope.start("test", |_| Ok(ToolResult::text("ok"))).unwrap();
    let second = scope.start("test", |_| Ok(ToolResult::text("ok"))).unwrap();
    assert_eq!(first.request_id, second.request_id);
    assert!(!first.started_at.is_empty());
    let batch = scope
        .wait(std::slice::from_ref(&first.execution_id), 100)
        .unwrap();
    scope.ack(&batch.batch_id).unwrap();
    assert!(scope
        .info(&first.execution_id)
        .unwrap()
        .completed_at
        .is_some());
}
#[test]
fn durable_artifacts_redact_registered_secrets() {
    atlas_agentic::diagnostics::protect_secret("execution-artifact-test-secret");
    let dir = tempfile::tempdir().unwrap();
    let manager = ToolExecutionManager::with_persistence(dir.path()).unwrap();
    let scope = manager.scope("owner");
    let run = scope
        .start("test", |reporter| {
            reporter.metadata(serde_json::json!({"value":"execution-artifact-test-secret"}));
            Ok(ToolResult::text("execution-artifact-test-secret"))
        })
        .unwrap();
    let batch = scope
        .wait(std::slice::from_ref(&run.execution_id), 100)
        .unwrap();
    scope.ack(&batch.batch_id).unwrap();
    assert!(!std::fs::read_to_string(snapshot_path(dir.path()))
        .unwrap()
        .contains("execution-artifact-test-secret"));
}
#[test]
fn independent_live_managers_share_storage_without_interrupting_each_other() {
    let dir = tempfile::tempdir().unwrap();
    let first = ToolExecutionManager::with_persistence(dir.path()).unwrap();
    let (ready, started) = std::sync::mpsc::channel();
    let (send, recv) = std::sync::mpsc::channel();
    let scope = first.scope("first");
    let run = scope
        .start("test", move |_| {
            ready.send(()).unwrap();
            recv.recv().unwrap();
            Ok(ToolResult::text("done"))
        })
        .unwrap();
    started.recv().unwrap();
    let second = ToolExecutionManager::with_persistence(dir.path()).unwrap();
    assert_eq!(scope.info(&run.execution_id).unwrap().status, "running");
    assert!(second.scope("first").list().unwrap().is_empty());
    send.send(()).unwrap();
}
#[test]
fn dormant_cli_archive_is_imported_by_an_existing_desktop_owner_binding() {
    let dir = tempfile::tempdir().unwrap();
    let desktop = ToolExecutionManager::with_persistence(dir.path()).unwrap();
    let id;
    {
        let cli = ToolExecutionManager::with_persistence(dir.path()).unwrap();
        let scope = cli.scope("saved");
        id = scope
            .start("test", |_| Ok(ToolResult::text("from CLI")))
            .unwrap()
            .execution_id;
        let batch = scope.wait(std::slice::from_ref(&id), 100).unwrap();
        scope.ack(&batch.batch_id).unwrap();
    }
    desktop.bind_persistence_owner("native", "saved").unwrap();
    assert_eq!(
        desktop
            .scope("native")
            .read(&id, None, Some(0))
            .unwrap()
            .text,
        "from CLI"
    );
}
#[test]
fn descendant_artifacts_follow_stable_owner_binding_and_deletion() {
    let dir = tempfile::tempdir().unwrap();
    let id;
    {
        let manager = ToolExecutionManager::with_persistence(dir.path()).unwrap();
        manager.bind_persistence_owner("native", "saved").unwrap();
        let child = manager.scope("native").child("run");
        id = child
            .start("test", |_| Ok(ToolResult::text("child result")))
            .unwrap()
            .execution_id;
        let batch = child.wait(std::slice::from_ref(&id), 100).unwrap();
        child.ack(&batch.batch_id).unwrap();
    }
    let manager = ToolExecutionManager::with_persistence(dir.path()).unwrap();
    manager.bind_persistence_owner("restored", "saved").unwrap();
    assert_eq!(
        manager
            .scope("restored")
            .child("run")
            .read(&id, None, Some(0))
            .unwrap()
            .text,
        "child result"
    );
    assert!(manager.scope("restored").claim().unwrap().is_none());
    manager.remove_persistence_owner("saved").unwrap();
    assert!(manager.scope("restored").child("run").info(&id).is_err());
    assert!(!std::fs::read_to_string(snapshot_path(dir.path()))
        .unwrap()
        .contains(&id));
}
#[test]
fn closing_a_child_preserves_committed_durable_artifacts_until_chat_deletion() {
    let dir = tempfile::tempdir().unwrap();
    let id;
    {
        let manager = ToolExecutionManager::with_persistence(dir.path()).unwrap();
        manager.bind_persistence_owner("native", "saved").unwrap();
        let child = manager.scope("native").child("run");
        id = child
            .start("test", |_| Ok(ToolResult::text("retained child result")))
            .unwrap()
            .execution_id;
        let batch = child.wait(std::slice::from_ref(&id), 100).unwrap();
        child.ack(&batch.batch_id).unwrap();
        child.close();
        assert!(child.info(&id).is_err());
        manager.bind_persistence_owner("native", "saved").unwrap();
    }
    let manager = ToolExecutionManager::with_persistence(dir.path()).unwrap();
    manager.bind_persistence_owner("restored", "saved").unwrap();
    assert_eq!(
        manager
            .scope("restored")
            .child("run")
            .read(&id, None, Some(0))
            .unwrap()
            .text,
        "retained child result"
    );
    manager.remove_persistence_owner("saved").unwrap();
    assert!(!std::fs::read_to_string(snapshot_path(dir.path()))
        .unwrap()
        .contains(&id));
}
#[test]
fn slow_persistence_does_not_block_execution_observation() {
    use fs2::FileExt;
    use std::time::Duration;
    let dir = tempfile::tempdir().unwrap();
    let manager = ToolExecutionManager::with_persistence(dir.path()).unwrap();
    let scope = manager.scope("owner");
    let run = scope
        .start("test", |_| Ok(ToolResult::text("done")))
        .unwrap();
    let batch = scope
        .wait(std::slice::from_ref(&run.execution_id), 100)
        .unwrap();
    scope.ack(&batch.batch_id).unwrap();
    let lock = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(dir.path().join("import.lock"))
        .unwrap();
    lock.lock_exclusive().unwrap();
    let writing = scope.clone();
    let id = run.execution_id.clone();
    let (started, ready) = std::sync::mpsc::channel();
    let writer = std::thread::spawn(move || {
        started.send(()).unwrap();
        writing.set_wakeup(
            &id,
            ExecutionPolicy {
                mode: WakeupMode::Lazy,
                interval_seconds: None,
            },
        )
    });
    ready.recv().unwrap();
    // Allow the writer to reach its bounded storage lease wait.
    std::thread::sleep(Duration::from_millis(50));
    let (send, recv) = std::sync::mpsc::channel();
    let reader = std::thread::spawn(move || {
        send.send(scope.info(&run.execution_id)).unwrap();
    });
    let observed = recv.recv_timeout(Duration::from_millis(150));
    lock.unlock().unwrap();
    writer.join().unwrap().unwrap();
    reader.join().unwrap();
    assert!(
        observed.is_ok(),
        "Reading execution metadata blocked on storage I/O"
    );
}
#[test]
fn concurrent_policy_updates_publish_the_latest_live_policy_without_tool_replay() {
    let dir = tempfile::tempdir().unwrap();
    let manager = ToolExecutionManager::with_persistence(dir.path()).unwrap();
    let scope = manager.scope("owner");
    let calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let count = calls.clone();
    let run = scope
        .start("test", move |_| {
            count.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Ok(ToolResult::text("done"))
        })
        .unwrap();
    let batch = scope
        .wait(std::slice::from_ref(&run.execution_id), 100)
        .unwrap();
    scope.ack(&batch.batch_id).unwrap();
    let mut workers = vec![];
    for index in 0..12 {
        let scope = scope.clone();
        let id = run.execution_id.clone();
        workers.push(std::thread::spawn(move || {
            scope.set_wakeup(
                &id,
                ExecutionPolicy {
                    mode: WakeupMode::Hard,
                    interval_seconds: Some(0.1 + index as f64 / 10.0),
                },
            )
        }));
    }
    for worker in workers {
        worker.join().unwrap().unwrap();
    }
    let live = scope.info(&run.execution_id).unwrap().wake_up;
    let snapshot: serde_json::Value =
        serde_json::from_slice(&std::fs::read(snapshot_path(dir.path())).unwrap()).unwrap();
    let saved = snapshot["records"]
        .as_array()
        .unwrap()
        .iter()
        .find(|record| record["id"] == run.execution_id)
        .unwrap();
    assert_eq!(saved["wake_up"], serde_json::to_value(live).unwrap());
    assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 1);
}
#[test]
fn recovered_descendant_results_are_read_only_and_notified_to_the_parent() {
    let dir = tempfile::tempdir().unwrap();
    let id;
    {
        let manager = ToolExecutionManager::with_persistence(dir.path()).unwrap();
        manager.bind_persistence_owner("native", "saved").unwrap();
        let child = manager.scope("native").child("run");
        id = child
            .start("test", |_| Ok(ToolResult::text("unread child result")))
            .unwrap()
            .execution_id;
        let batch = child.wait(std::slice::from_ref(&id), 100).unwrap();
        assert_eq!(batch.events[0].status, "succeeded");
    }
    let manager = ToolExecutionManager::with_persistence(dir.path()).unwrap();
    manager.bind_persistence_owner("restored", "saved").unwrap();
    let parent = manager.scope("restored");
    let batch = parent
        .claim()
        .unwrap()
        .expect("Recovered child result must reach its parent");
    assert_eq!(batch.events[0].execution_id, id);
    let info = parent.info(&id).unwrap();
    assert!(info.archived);
    assert!(parent
        .list()
        .unwrap()
        .iter()
        .any(|entry| entry.execution_id == id));
    assert_eq!(
        parent.read(&id, None, Some(0)).unwrap().text,
        "unread child result"
    );
    assert!(parent.cancel(&id).is_err());
    assert!(parent.set_wakeup(&id, ExecutionPolicy::default()).is_err());
    parent.ack(&batch.batch_id).unwrap();
    assert!(parent.claim().unwrap().is_none());
}
#[test]
fn acknowledged_closed_child_archives_share_the_configured_retention_quota() {
    use atlas_agentic::executions::PersistenceLimits;
    let dir = tempfile::tempdir().unwrap();
    let manager = ToolExecutionManager::with_persistence_options(
        dir.path(),
        PersistenceLimits {
            max_records: 1,
            max_bytes: 1024 * 1024,
        },
    )
    .unwrap();
    let child = manager.scope("owner").child("run");
    let first = child
        .start("test", |_| Ok(ToolResult::text("old evidence")))
        .unwrap();
    let batch = child
        .wait(std::slice::from_ref(&first.execution_id), 100)
        .unwrap();
    child.ack(&batch.batch_id).unwrap();
    child.close();
    let parent = manager.scope("owner");
    let second = parent
        .start("test", |_| Ok(ToolResult::text("new evidence")))
        .unwrap();
    let batch = parent
        .wait(std::slice::from_ref(&second.execution_id), 100)
        .unwrap();
    parent.ack(&batch.batch_id).unwrap();
    let snapshot: serde_json::Value =
        serde_json::from_slice(&std::fs::read(snapshot_path(dir.path())).unwrap()).unwrap();
    let records = snapshot["records"].as_array().unwrap();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0]["id"], second.execution_id);
}
#[test]
fn retired_child_notifications_do_not_pin_execution_retention() {
    use atlas_agentic::executions::PersistenceLimits;
    let dir = tempfile::tempdir().unwrap();
    let manager = ToolExecutionManager::with_persistence_options(
        dir.path(),
        PersistenceLimits {
            max_records: 1,
            max_bytes: 1024 * 1024,
        },
    )
    .unwrap();
    let child = manager.scope("owner").child("run");
    let first = child
        .start("test", |_| Ok(ToolResult::text("old evidence")))
        .unwrap();
    child.wait(&[first.execution_id], 100).unwrap();
    child.close();
    let parent = manager.scope("owner");
    let next = parent
        .start("test", |_| Ok(ToolResult::text("new work")))
        .unwrap();
    assert!(parent.info(&next.execution_id).is_ok());
}
