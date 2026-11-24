use atlas_agentic::{
    conversations::ConversationId,
    observability::ObservabilityConfig,
    sessions::{ContextCursor, SessionStore},
};
use chrono::DateTime;
use serde_json::json;
use std::fs;
fn at(value: &str) -> chrono::DateTime<chrono::FixedOffset> {
    DateTime::parse_from_rfc3339(value).unwrap()
}
#[test]
fn session_is_created_on_first_send_and_resumes_its_original_dated_file() {
    let data = tempfile::tempdir().unwrap();
    let store = SessionStore::new(data.path(), ObservabilityConfig::default().session).unwrap();
    let journal = store
        .open(
            None,
            Some("native-draft"),
            at("2026-11-23T08:09:10.123456+07:00"),
        )
        .unwrap();
    let id = journal.id();
    let directory = journal.path().to_owned();
    assert!(directory.ends_with(format!("2026/11/23/080910123456_{id}.json")));
    assert!(directory.is_file());
    assert_eq!(
        fs::read_dir(directory.parent().unwrap()).unwrap().count(),
        1
    );
    let turn = journal.start_turn("hello").unwrap();
    journal
        .record(
            &turn,
            "tool_call",
            json!({"tool":"read_file","arguments":{"path":"README.md"}}),
        )
        .unwrap();
    let cursor = ContextCursor {
        history_message_start: 4,
        checkpoint_id: Some("checkpoint".into()),
        prepared_event: None,
    };
    journal
        .prepare_context(&turn, cursor, json!({"input":"next"}))
        .unwrap();
    journal.finish_turn(&turn, "failed").unwrap();
    let before = journal.events().unwrap();
    assert_eq!(before.len(), 4);
    assert!(before
        .windows(2)
        .all(|w| w[0].sequence + 1 == w[1].sequence));
    assert!(before.iter().all(|e| e.session_id == id));
    drop(journal);
    let journal = store
        .open(None, Some("native-draft"), at("2027-01-01T00:00:00+07:00"))
        .unwrap();
    assert_eq!(journal.id(), id);
    assert_eq!(journal.path(), directory);
    assert_eq!(
        journal
            .metadata()
            .unwrap()
            .context_cursor
            .history_message_start,
        4
    );
    assert_eq!(
        journal.metadata().unwrap().context_cursor.prepared_event,
        Some(3)
    );
    let turn = journal.start_turn("retry").unwrap();
    journal.finish_turn(&turn, "succeeded").unwrap();
    assert_eq!(journal.events().unwrap().len(), 6);
    let other = store.open(Some(&id), None, at("2026-11-23T00:00:00Z"));
    assert!(
        other.is_err(),
        "session lease must reject concurrent writers"
    );
}
#[test]
fn session_json_is_updated_live_and_recovers_an_unfinished_turn() {
    let data = tempfile::tempdir().unwrap();
    let store = SessionStore::new(data.path(), ObservabilityConfig::default().session).unwrap();
    let journal = store.open(None, None, at("2026-11-23T23:59:59Z")).unwrap();
    let id = journal.id();
    let dir = journal.path().to_owned();
    let turn = journal.start_turn("task").unwrap();
    journal
        .record(
            &turn,
            "wakeup",
            json!({"executionId":"example","status":"running"}),
        )
        .unwrap();
    let event = dir.clone();
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&fs::read(event).unwrap()).unwrap()["events"]
            [1]["kind"],
        "wakeup"
    );
    drop(journal);
    fs::write(dir.with_extension("incomplete.tmp"), "partial").unwrap();
    let journal = store
        .open(Some(&id), None, at("2026-11-24T00:00:01Z"))
        .unwrap();
    assert_eq!(
        journal.events().unwrap().last().unwrap().kind,
        "turn_recovered"
    );
    assert!(journal.metadata().unwrap().active_turn.is_none());
    assert_eq!(journal.path(), dir);
}
#[test]
fn session_limits_and_paths_are_validated_and_corruption_is_not_silently_replaced() {
    let data = tempfile::tempdir().unwrap();
    let mut config = ObservabilityConfig::default().session;
    config.directory = "../outside".into();
    assert!(SessionStore::new(data.path(), config).is_err());
    let store = SessionStore::new(data.path(), ObservabilityConfig::default().session).unwrap();
    let journal = store.open(None, None, at("2026-11-23T00:00:00Z")).unwrap();
    let id = journal.id();
    let dir = journal.path().to_owned();
    drop(journal);
    fs::write(&dir, r#"{"version":999}"#).unwrap();
    assert!(store
        .open(Some(&id), None, at("2026-11-23T00:00:00Z"))
        .is_err());
}
#[test]
fn journal_redacts_credentials_and_explicitly_marks_bounded_text() {
    let data = tempfile::tempdir().unwrap();
    let mut config = ObservabilityConfig::default().session;
    config.max_text_chars = 32;
    let store = SessionStore::new(data.path(), config).unwrap();
    let journal = store.open(None, None, at("2026-11-23T00:00:00Z")).unwrap();
    let turn = journal.start_turn("hello").unwrap();
    atlas_agentic::diagnostics::protect_secret("journal-test-credential");
    journal.record(&turn,"tool_result",json!({"output":"journal-test-credential","detail":"x".repeat(100),"apiKey":"do-not-store"})).unwrap();
    let events = journal.events().unwrap();
    let event = events.last().unwrap();
    let text = serde_json::to_string(event).unwrap();
    assert!(!text.contains("journal-test-credential"));
    assert!(!text.contains("do-not-store"));
    assert!(event.truncated);
}
#[cfg(unix)]
#[test]
fn session_files_are_private_and_symlinked_session_files_are_rejected() {
    use std::os::unix::fs::{symlink, PermissionsExt};
    let data = tempfile::tempdir().unwrap();
    let store = SessionStore::new(data.path(), ObservabilityConfig::default().session).unwrap();
    let journal = store
        .open(
            Some(&ConversationId::new()),
            None,
            at("2026-11-23T00:00:00Z"),
        )
        .unwrap();
    let turn = journal.start_turn("hello").unwrap();
    let dir = journal.path().to_owned();
    assert_eq!(
        fs::metadata(&dir).unwrap().permissions().mode() & 0o777,
        0o600
    );
    assert_eq!(
        fs::metadata(dir.parent().unwrap())
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o700
    );
    let outside = tempfile::tempdir().unwrap();
    fs::rename(&dir, dir.with_extension("old")).unwrap();
    symlink(outside.path().join("victim.json"), &dir).unwrap();
    assert!(journal.record(&turn, "tool_call", json!({})).is_err());
    assert_eq!(fs::read_dir(outside.path()).unwrap().count(), 0);
}

#[test]
fn session_archive_moves_trace_and_can_restore_it_for_a_failed_conversation_delete() {
    let data = tempfile::tempdir().unwrap();
    let store = SessionStore::new(data.path(), ObservabilityConfig::default().session).unwrap();
    let journal = store
        .open(None, Some("draft"), at("2026-11-23T00:00:00Z"))
        .unwrap();
    let id = journal.id();
    let original = journal.path().to_owned();
    let turn = journal.start_turn("task").unwrap();
    journal.finish_turn(&turn, "succeeded").unwrap();
    assert!(
        store.archive(&id).is_err(),
        "active session lease must protect its files"
    );
    drop(journal);
    let archive = store.archive(&id).unwrap().unwrap();
    assert!(!original.exists());
    assert!(store.find(&id).unwrap().is_none());
    archive.restore().unwrap();
    assert_eq!(store.find(&id).unwrap().unwrap(), original);
    let journal = store
        .open(None, Some("draft"), at("2027-01-01T00:00:00Z"))
        .unwrap();
    assert_eq!(journal.id(), id);
}

#[test]
fn embedded_json_credentials_are_redacted_in_context_continuations() {
    let data = tempfile::tempdir().unwrap();
    let store = SessionStore::new(data.path(), ObservabilityConfig::default().session).unwrap();
    let journal = store.open(None, None, at("2026-11-23T00:00:00Z")).unwrap();
    let turn = journal.start_turn("task").unwrap();
    journal.record(&turn,"context_prepared",json!({"cursor":ContextCursor::default(),"selection":{"input":r#"{"tool_results":[{"args":{"password":"embedded-secret"}}]}"#}})).unwrap();
    let events = serde_json::to_string(&journal.events().unwrap()).unwrap();
    assert!(!events.contains("embedded-secret"));
}

#[test]
fn index_growth_is_rejected_before_publishing_an_unreadable_index() {
    let data = tempfile::tempdir().unwrap();
    let mut config = ObservabilityConfig::default().session;
    config.max_index_bytes = 1024;
    let store = SessionStore::new(data.path(), config).unwrap();
    let mut first = None;
    let mut rejected = false;
    for n in 0..30 {
        match store.open(
            None,
            Some(&format!("draft-{n}")),
            at("2026-11-23T00:00:00Z"),
        ) {
            Ok(journal) => {
                first.get_or_insert(journal.id());
            }
            Err(_) => {
                rejected = true;
                break;
            }
        }
    }
    assert!(rejected);
    let id = first.unwrap();
    assert!(store.find(&id).unwrap().is_some());
    assert!(store
        .open(Some(&id), None, at("2026-11-23T00:00:00Z"))
        .is_ok());
}

#[test]
fn simultaneous_sessions_have_distinct_json_files_in_the_same_day() {
    let data = tempfile::tempdir().unwrap();
    let store = SessionStore::new(data.path(), ObservabilityConfig::default().session).unwrap();
    let first = store.open(None, None, at("2026-11-24T08:00:00Z")).unwrap();
    let second = store.open(None, None, at("2026-11-24T08:00:00Z")).unwrap();
    assert_ne!(first.path(), second.path());
    let day = data.path().join("sessions/2026/11/24");
    let files: Vec<_> = fs::read_dir(day)
        .unwrap()
        .map(|e| e.unwrap().path())
        .collect();
    assert_eq!(files.len(), 2);
    assert!(files
        .iter()
        .all(|p| p.is_file() && p.extension().unwrap() == "json"));
}

#[test]
fn aggregate_limit_preserves_last_complete_session_and_blocks_further_writes() {
    let data = tempfile::tempdir().unwrap();
    let mut config = ObservabilityConfig::default().session;
    config.max_event_bytes = 1024;
    config.max_text_chars = 128;
    config.max_session_bytes = 2048;
    let store = SessionStore::new(data.path(), config).unwrap();
    let journal = store.open(None, None, at("2026-11-24T08:00:00Z")).unwrap();
    let turn = journal.start_turn("hello").unwrap();
    let mut last_good = fs::read(journal.path()).unwrap();
    let mut failed = false;
    for _ in 0..20 {
        if journal
            .record(&turn, "progress", json!({"text":"x".repeat(100)}))
            .is_err()
        {
            failed = true;
            break;
        }
        last_good = fs::read(journal.path()).unwrap();
    }
    assert!(failed);
    assert_eq!(fs::read(journal.path()).unwrap(), last_good);
    assert!(journal.check().is_err());
    assert!(journal.record(&turn, "progress", json!({})).is_err());
}

fn legacy_session(
    data: &std::path::Path,
    corrupt: bool,
) -> (SessionStore, ConversationId, std::path::PathBuf) {
    let id = ConversationId::new();
    let date = at("2026-11-23T08:09:10.123456+07:00");
    let root = data.join("sessions");
    let dir = root.join(format!("2026/11/23/080910123456_{id}"));
    fs::create_dir_all(dir.join("events")).unwrap();
    fs::write(dir.join("session.lock"), "").unwrap();
    fs::write(
        root.join("index.json"),
        serde_json::to_vec(&json!({
            "version":1,"sessions":{id.to_string():date},"drafts":{}
        }))
        .unwrap(),
    )
    .unwrap();
    fs::write(
        dir.join("session.json"),
        serde_json::to_vec(&json!({
            "version":1,"id":id,"firstSentAt":date,"lastSequence":0,
            "activeTurn":null,"lastFinishedTurn":null,
            "contextCursor":{"historyMessageStart":0,"checkpointId":null,"preparedEvent":null}
        }))
        .unwrap(),
    )
    .unwrap();
    fs::write(
        dir.join("events/00000000000000000001.json"),
        if corrupt {
            b"invalid".to_vec()
        } else {
            serde_json::to_vec(&json!({"version":1,"sessionId":id,"turnId":"old-turn",
            "sequence":1,"timestamp":"2026-11-23T01:09:10Z","kind":"turn_started",
            "data":{"input":"original request"},"truncated":false}))
            .unwrap()
        },
    )
    .unwrap();
    let store = SessionStore::new(data, ObservabilityConfig::default().session).unwrap();
    (store, id, dir)
}

#[test]
fn legacy_journal_converts_on_open_without_losing_events_and_recovers_interruption() {
    let data = tempfile::tempdir().unwrap();
    let (store, id, old) = legacy_session(data.path(), false);
    // An older Atlas writer must also prevent conversion.
    let lock = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(old.join("session.lock"))
        .unwrap();
    fs2::FileExt::lock_exclusive(&lock).unwrap();
    assert!(store
        .open(Some(&id), None, at("2026-11-24T00:00:00Z"))
        .is_err());
    drop(lock);
    let journal = store
        .open(Some(&id), None, at("2026-11-24T00:00:00Z"))
        .unwrap();
    assert!(journal
        .path()
        .ends_with(format!("2026/11/23/080910123456_{id}.json")));
    assert!(!old.exists());
    assert_eq!(store.find(&id).unwrap().unwrap(), journal.path());
    let value: serde_json::Value =
        serde_json::from_slice(&fs::read(journal.path()).unwrap()).unwrap();
    assert_eq!(value["events"][0]["data"]["input"], "original request");
    assert_eq!(value["events"][1]["kind"], "turn_recovered");
    drop(journal);
    assert_eq!(
        store
            .open(Some(&id), None, at("2026-11-24T00:00:00Z"))
            .unwrap()
            .events()
            .unwrap()
            .len(),
        2
    );
}

#[test]
fn corrupt_legacy_journal_is_retained_without_publishing_a_replacement() {
    let data = tempfile::tempdir().unwrap();
    let (store, id, old) = legacy_session(data.path(), true);
    assert!(store
        .open(Some(&id), None, at("2026-11-24T00:00:00Z"))
        .is_err());
    assert!(old.join("events/00000000000000000001.json").exists());
    assert!(!old.with_extension("json").exists());
}

#[test]
fn unopened_legacy_journal_can_be_archived_and_restored() {
    let data = tempfile::tempdir().unwrap();
    let (store, id, old) = legacy_session(data.path(), false);
    let archive = store.archive(&id).unwrap().unwrap();
    assert!(!old.exists());
    archive.restore().unwrap();
    assert!(old.join("session.json").is_file());
    assert!(store
        .open(Some(&id), None, at("2026-11-24T00:00:00Z"))
        .is_ok());
}

#[test]
fn dated_directory_contains_complete_session_json_without_subdirectories() {
    let data = tempfile::tempdir().unwrap();
    let store = SessionStore::new(data.path(), ObservabilityConfig::default().session).unwrap();
    let journal = store.open(None, None, at("2026-11-24T08:00:00Z")).unwrap();
    journal.start_turn("hello").unwrap();
    let files: Vec<_> = fs::read_dir(data.path().join("sessions/2026/11/24"))
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect();
    assert_eq!(files.len(), 1);
    assert!(
        files[0].is_file(),
        "each session must be one file directly in the date directory"
    );
    let doc: serde_json::Value = serde_json::from_slice(&fs::read(&files[0]).unwrap()).unwrap();
    assert_eq!(doc["events"][0]["data"]["input"], "hello");
}

#[test]
fn interrupted_conversion_reopens_published_file_and_removes_legacy_copy() {
    let data = tempfile::tempdir().unwrap();
    let (store, id, old) = legacy_session(data.path(), false);
    let metadata: serde_json::Value =
        serde_json::from_slice(&fs::read(old.join("session.json")).unwrap()).unwrap();
    let event: serde_json::Value =
        serde_json::from_slice(&fs::read(old.join("events/00000000000000000001.json")).unwrap())
            .unwrap();
    fs::write(
        old.with_extension("json"),
        serde_json::to_vec(&json!({"version":2,"metadata":metadata,"events":[event]})).unwrap(),
    )
    .unwrap();
    let journal = store
        .open(Some(&id), None, at("2026-11-24T00:00:00Z"))
        .unwrap();
    assert!(!old.exists());
    assert_eq!(journal.events().unwrap().len(), 2);
}

#[test]
fn failed_conversion_preserves_legacy_source_and_does_not_publish_oversized_json() {
    let data = tempfile::tempdir().unwrap();
    let (_, id, old) = legacy_session(data.path(), false);
    let path = old.join("events/00000000000000000001.json");
    let mut event: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    event["data"]["input"] = json!("x".repeat(400));
    fs::write(&path, serde_json::to_vec(&event).unwrap()).unwrap();
    let mut config = ObservabilityConfig::default().session;
    config.max_event_bytes = 1024;
    config.max_session_bytes = 1024;
    config.max_text_chars = 128;
    let store = SessionStore::new(data.path(), config).unwrap();
    assert!(store
        .open(Some(&id), None, at("2026-11-24T00:00:00Z"))
        .is_err());
    assert!(path.is_file());
    assert!(!old.with_extension("json").exists());
}

#[test]
fn divergent_legacy_copy_is_not_deleted_after_interrupted_conversion() {
    let data = tempfile::tempdir().unwrap();
    let (store, id, old) = legacy_session(data.path(), false);
    let metadata: serde_json::Value =
        serde_json::from_slice(&fs::read(old.join("session.json")).unwrap()).unwrap();
    let mut event: serde_json::Value =
        serde_json::from_slice(&fs::read(old.join("events/00000000000000000001.json")).unwrap())
            .unwrap();
    event["data"]["input"] = json!("different request");
    fs::write(
        old.with_extension("json"),
        serde_json::to_vec(&json!({"version":2,"metadata":metadata,"events":[event]})).unwrap(),
    )
    .unwrap();
    let before = fs::read(old.with_extension("json")).unwrap();
    assert!(store
        .open(Some(&id), None, at("2026-11-24T00:00:00Z"))
        .is_err());
    assert!(old.is_dir());
    assert_eq!(fs::read(old.with_extension("json")).unwrap(), before);
}

#[test]
fn archive_retains_both_formats_until_interrupted_conversion_is_finished() {
    let data = tempfile::tempdir().unwrap();
    let (store, id, old) = legacy_session(data.path(), false);
    let metadata: serde_json::Value =
        serde_json::from_slice(&fs::read(old.join("session.json")).unwrap()).unwrap();
    let event: serde_json::Value =
        serde_json::from_slice(&fs::read(old.join("events/00000000000000000001.json")).unwrap())
            .unwrap();
    let file = old.with_extension("json");
    fs::write(
        &file,
        serde_json::to_vec(&json!({"version":2,"metadata":metadata,"events":[event]})).unwrap(),
    )
    .unwrap();
    assert!(store.archive(&id).is_err());
    assert!(file.is_file());
    assert!(old.is_dir());
    assert!(store.find(&id).unwrap().is_some());
    drop(
        store
            .open(Some(&id), None, at("2026-11-24T00:00:00Z"))
            .unwrap(),
    );
    let archive = store.archive(&id).unwrap().unwrap();
    assert!(!file.exists());
    assert!(!old.exists());
    assert!(store
        .open(Some(&id), None, at("2026-11-24T00:00:00Z"))
        .is_err());
    archive.restore().unwrap();
    assert!(file.is_file());
    assert_eq!(
        store
            .open(Some(&id), None, at("2026-11-24T00:00:00Z"))
            .unwrap()
            .events()
            .unwrap()
            .len(),
        2
    );
}
