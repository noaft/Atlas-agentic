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
fn session_is_created_on_first_send_and_resumes_its_original_dated_directory() {
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
    let directory = journal.directory().to_owned();
    assert!(directory.ends_with(format!("2026/11/23/080910123456_{id}")));
    assert!(directory.join("session.json").is_file());
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
    assert_eq!(journal.directory(), directory);
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
fn events_are_live_atomic_json_documents_and_recover_an_unfinished_turn() {
    let data = tempfile::tempdir().unwrap();
    let store = SessionStore::new(data.path(), ObservabilityConfig::default().session).unwrap();
    let journal = store.open(None, None, at("2026-11-23T23:59:59Z")).unwrap();
    let id = journal.id();
    let dir = journal.directory().to_owned();
    let turn = journal.start_turn("task").unwrap();
    journal
        .record(
            &turn,
            "wakeup",
            json!({"executionId":"example","status":"running"}),
        )
        .unwrap();
    let event = dir.join("events/00000000000000000002.json");
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&fs::read(event).unwrap()).unwrap()["kind"],
        "wakeup"
    );
    drop(journal);
    fs::write(dir.join("events/.incomplete.tmp"), "partial").unwrap();
    let journal = store
        .open(Some(&id), None, at("2026-11-24T00:00:01Z"))
        .unwrap();
    assert_eq!(
        journal.events().unwrap().last().unwrap().kind,
        "turn_recovered"
    );
    assert!(journal.metadata().unwrap().active_turn.is_none());
    assert_eq!(journal.directory(), dir);
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
    let dir = journal.directory().to_owned();
    drop(journal);
    fs::write(dir.join("session.json"), r#"{"version":999}"#).unwrap();
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
fn session_files_are_private_and_symlinked_event_directories_are_rejected() {
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
    let dir = journal.directory().to_owned();
    assert_eq!(
        fs::metadata(dir.join("session.json"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
    assert_eq!(
        fs::metadata(&dir).unwrap().permissions().mode() & 0o777,
        0o700
    );
    let outside = tempfile::tempdir().unwrap();
    fs::rename(dir.join("events"), dir.join("old-events")).unwrap();
    symlink(outside.path(), dir.join("events")).unwrap();
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
    let original = journal.directory().to_owned();
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
