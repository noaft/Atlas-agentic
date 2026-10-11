use std::{
    env, fs, io,
    io::Write,
    path::PathBuf,
    sync::{
        atomic::{AtomicUsize, Ordering},
        mpsc, Arc, Mutex, OnceLock,
    },
    thread,
    time::Duration,
};

use atlas_agentic::conversations::{
    ActivityTurn, ConversationId, ConversationMessage, ConversationStore, MessageRole,
};
use atlas_agentic::permissions::PermissionMode;
use chrono::{Duration as ChronoDuration, Utc};
use fs2::FileExt;
use serde_json::{json, Value};
use tempfile::tempdir;

const FALLBACK_TITLE: &str = "Untitled conversation";

fn activity_turn(id: &str) -> ActivityTurn {
    ActivityTurn {
        version: 1,
        turn_id: id.to_string(),
        assistant_message_index: usize::MAX,
        records: Vec::new(),
    }
}

fn read_index(root: &std::path::Path) -> Value {
    serde_json::from_str(&fs::read_to_string(root.join("conversations.json")).unwrap()).unwrap()
}

fn env_lock() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(())).lock().unwrap()
}

fn lock_store_exclusively(root: &std::path::Path) -> fs::File {
    fs::create_dir_all(root).unwrap();
    let lock = fs::OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .truncate(false)
        .open(root.join("conversations.lock"))
        .unwrap();
    lock.lock_exclusive().unwrap();
    lock
}

fn store_failing_index_publication(
    root: &std::path::Path,
    failure_call: usize,
) -> ConversationStore {
    let publication_count = Arc::new(AtomicUsize::new(0));
    ConversationStore::at_with_index_publisher(root.to_path_buf(), move |source, destination| {
        let call = publication_count.fetch_add(1, Ordering::SeqCst) + 1;
        if call == failure_call {
            Err(io::Error::other("injected index publication failure"))
        } else {
            fs::rename(source, destination)
        }
    })
}

#[test]
fn create_turn_writes_versioned_index_and_two_jsonl_messages() {
    let data = tempdir().unwrap();
    let store = ConversationStore::at(data.path().to_path_buf());

    let metadata = store
        .create_turn("first line\nsecond line ❤", "reply\n日本語")
        .unwrap();

    assert_eq!(metadata.title, FALLBACK_TITLE);
    assert!(!metadata.title_attempted);
    assert_eq!(metadata.message_count, 2);
    assert_eq!(metadata.created_at, metadata.updated_at);
    assert_eq!(metadata.workspace_path, None);
    assert_eq!(metadata.permission_mode, None);

    let index = read_index(data.path());
    assert_eq!(index["version"], 1);
    assert_eq!(index["conversations"].as_array().unwrap().len(), 1);
    assert_eq!(index["conversations"][0]["id"], metadata.id.to_string());

    let history_path = data
        .path()
        .join("conversations")
        .join(format!("{}.jsonl", metadata.id));
    let raw_history = fs::read_to_string(&history_path).unwrap();
    assert_eq!(raw_history.lines().count(), 2);
    assert!(!raw_history.contains("first line\nsecond line"));

    let conversation = store.load(&metadata.id).unwrap();
    assert_eq!(conversation.metadata, metadata);
    assert_eq!(conversation.messages.len(), 2);
    assert_eq!(conversation.messages[0].role, MessageRole::User);
    assert_eq!(
        conversation.messages[0].content,
        "first line\nsecond line ❤"
    );
    assert_eq!(conversation.messages[1].role, MessageRole::Assistant);
    assert_eq!(conversation.messages[1].content, "reply\n日本語");
}

#[test]
fn successful_turn_persists_and_loads_ordered_activity() {
    let data = tempdir().unwrap();
    let store = ConversationStore::at(data.path().to_path_buf());
    let metadata = store
        .create_turn_with_activity("first", "answer", None, None, activity_turn("turn-1"))
        .unwrap();
    store
        .append_turn_with_activity(
            &metadata.id,
            "second",
            "answer",
            None,
            None,
            activity_turn("turn-2"),
        )
        .unwrap();

    let loaded = store.load(&metadata.id).unwrap();
    assert_eq!(
        loaded
            .activities
            .iter()
            .map(|turn| (turn.turn_id.as_str(), turn.assistant_message_index))
            .collect::<Vec<_>>(),
        vec![("turn-1", 1), ("turn-2", 3)]
    );
    let path = data
        .path()
        .join("activities")
        .join(format!("{}.jsonl", metadata.id));
    assert_eq!(fs::read_to_string(path).unwrap().lines().count(), 2);
}

#[test]
fn missing_activity_sidecar_loads_as_empty() {
    let data = tempdir().unwrap();
    let store = ConversationStore::at(data.path().to_path_buf());
    let metadata = store.create_turn("user", "assistant").unwrap();

    assert!(store.load(&metadata.id).unwrap().activities.is_empty());
}

#[test]
fn tool_free_turn_persists_an_empty_activity_batch_for_association() {
    let data = tempdir().unwrap();
    let store = ConversationStore::at(data.path().to_path_buf());
    let metadata = store
        .create_turn_with_activity("user", "assistant", None, None, activity_turn("tool-free"))
        .unwrap();

    let loaded = store.load(&metadata.id).unwrap();
    assert_eq!(loaded.activities.len(), 1);
    assert_eq!(loaded.activities[0].assistant_message_index, 1);
    assert!(loaded.activities[0].records.is_empty());
}

#[test]
fn activity_batch_beyond_durable_message_count_is_ignored() {
    let data = tempdir().unwrap();
    let store = ConversationStore::at(data.path().to_path_buf());
    let metadata = store
        .create_turn_with_activity("user", "assistant", None, None, activity_turn("durable"))
        .unwrap();
    let path = data
        .path()
        .join("activities")
        .join(format!("{}.jsonl", metadata.id));
    let mut orphan = activity_turn("orphan");
    orphan.assistant_message_index = 99;
    let mut file = fs::OpenOptions::new().append(true).open(path).unwrap();
    writeln!(file, "{}", serde_json::to_string(&orphan).unwrap()).unwrap();

    let loaded = store.load(&metadata.id).unwrap();
    assert_eq!(loaded.activities.len(), 1);
    assert_eq!(loaded.activities[0].turn_id, "durable");
}

#[test]
fn activity_write_failure_leaves_messages_and_metadata_unchanged() {
    let data = tempdir().unwrap();
    fs::create_dir_all(data.path()).unwrap();
    fs::write(data.path().join("activities"), "blocks the directory").unwrap();
    let store = ConversationStore::at(data.path().to_path_buf());

    let error = store
        .create_turn_with_activity("user", "assistant", None, None, activity_turn("blocked"))
        .unwrap_err();

    assert!(error.contains("activities"), "{error}");
    assert!(!data.path().join("conversations.json").exists());
    assert_eq!(
        fs::read_dir(data.path().join("conversations"))
            .unwrap()
            .count(),
        0
    );
}

#[test]
fn delete_moves_activity_to_trash_when_present() {
    let data = tempdir().unwrap();
    let store = ConversationStore::at(data.path().to_path_buf());
    let metadata = store
        .create_turn_with_activity("user", "assistant", None, None, activity_turn("turn"))
        .unwrap();
    let source = data
        .path()
        .join("activities")
        .join(format!("{}.jsonl", metadata.id));

    store.delete(&metadata.id).unwrap();

    assert!(!source.exists());
    assert!(data
        .path()
        .join("trash")
        .join(format!("{}.activity.jsonl", metadata.id))
        .is_file());
}

#[test]
fn delete_without_activity_remains_compatible() {
    let data = tempdir().unwrap();
    let store = ConversationStore::at(data.path().to_path_buf());
    let metadata = store.create_turn("user", "assistant").unwrap();

    let history = store.delete(&metadata.id).unwrap();

    assert!(history.is_file());
    assert!(store.list().unwrap().is_empty());
}

#[test]
fn version_one_index_without_workspace_path_loads_as_unassigned() {
    let data = tempdir().unwrap();
    let store = ConversationStore::at(data.path().to_path_buf());
    let metadata = store.create_turn("user", "assistant").unwrap();
    let path = data.path().join("conversations.json");
    let mut index = read_index(data.path());
    index["conversations"][0]
        .as_object_mut()
        .unwrap()
        .remove("workspacePath");
    fs::write(path, serde_json::to_vec_pretty(&index).unwrap()).unwrap();

    assert_eq!(
        store.load(&metadata.id).unwrap().metadata.workspace_path,
        None
    );
    assert_eq!(
        store.load(&metadata.id).unwrap().metadata.permission_mode,
        None
    );
}

#[test]
fn permission_mode_is_assigned_on_success_and_updates_only_one_conversation() {
    let data = tempdir().unwrap();
    let store = ConversationStore::at(data.path().to_path_buf());
    let first = store
        .create_turn_with_workspace_and_permission(
            "user",
            "assistant",
            None,
            Some(PermissionMode::Workspace),
        )
        .unwrap();
    let second = store.create_turn("other", "answer").unwrap();

    assert_eq!(first.permission_mode, Some(PermissionMode::Workspace));
    let updated = store
        .set_permission_mode(&first.id, PermissionMode::FullAccess)
        .unwrap();

    assert_eq!(updated.permission_mode, Some(PermissionMode::FullAccess));
    assert_eq!(
        store.load(&second.id).unwrap().metadata.permission_mode,
        None
    );
}

#[test]
fn legacy_append_assigns_permission_only_after_successful_publication() {
    let data = tempdir().unwrap();
    let root = data.path().join("data");
    let initial = ConversationStore::at(root.clone())
        .create_turn("user", "assistant")
        .unwrap();
    let failing = store_failing_index_publication(&root, 1);

    let error = failing
        .append_turn_with_workspace_and_permission_if_missing(
            &initial.id,
            "next",
            "answer",
            None,
            Some(PermissionMode::Workspace),
        )
        .unwrap_err();

    assert!(
        error.contains("injected index publication failure"),
        "{error}"
    );
    assert_eq!(
        ConversationStore::at(root)
            .load(&initial.id)
            .unwrap()
            .metadata
            .permission_mode,
        None
    );
}

#[test]
fn creation_stores_a_canonical_absolute_workspace_directory() {
    let data = tempdir().unwrap();
    let workspace = data.path().join("workspace");
    fs::create_dir(&workspace).unwrap();
    let store = ConversationStore::at(data.path().join("data"));

    let metadata = store
        .create_turn_with_workspace("user", "assistant", Some(&workspace))
        .unwrap();

    assert_eq!(
        metadata.workspace_path,
        Some(workspace.canonicalize().unwrap())
    );
    assert!(metadata.workspace_path.unwrap().is_absolute());
}

#[test]
fn assigning_workspace_preserves_conversation_except_for_workspace_and_update_time() {
    let data = tempdir().unwrap();
    let first_workspace = data.path().join("first");
    let second_workspace = data.path().join("second");
    fs::create_dir(&first_workspace).unwrap();
    fs::create_dir(&second_workspace).unwrap();
    let store = ConversationStore::at(data.path().join("data"));
    let created = store
        .create_turn_with_workspace("user", "assistant", Some(&first_workspace))
        .unwrap();
    let titled = store.rename(&created.id, "Chosen title").unwrap();
    let before = store.load(&created.id).unwrap();
    thread::sleep(Duration::from_millis(2));

    let assigned = store
        .assign_workspace(&created.id, &second_workspace)
        .unwrap();

    assert_eq!(assigned.id, titled.id);
    assert_eq!(assigned.title, titled.title);
    assert_eq!(assigned.title_attempted, titled.title_attempted);
    assert_eq!(assigned.created_at, titled.created_at);
    assert_eq!(assigned.message_count, titled.message_count);
    assert!(assigned.updated_at > titled.updated_at);
    assert_eq!(
        assigned.workspace_path,
        Some(second_workspace.canonicalize().unwrap())
    );
    let after = store.load(&created.id).unwrap();
    assert_eq!(after.messages, before.messages);
    assert_eq!(after.metadata, assigned);
}

#[test]
fn workspace_validation_rejects_missing_paths_and_regular_files_without_mutation() {
    let data = tempdir().unwrap();
    let store = ConversationStore::at(data.path().join("data"));
    let metadata = store.create_turn("user", "assistant").unwrap();
    let before = store.load(&metadata.id).unwrap();
    let missing = data.path().join("missing");
    let file = data.path().join("file.txt");
    fs::write(&file, "not a directory").unwrap();

    assert!(store.assign_workspace(&metadata.id, &missing).is_err());
    assert!(store.assign_workspace(&metadata.id, &file).is_err());
    assert_eq!(store.load(&metadata.id).unwrap(), before);
}

#[test]
fn workspace_removed_after_selection_fails_creation_without_metadata() {
    let data = tempdir().unwrap();
    let workspace = data.path().join("selected");
    fs::create_dir(&workspace).unwrap();
    let selected = workspace.clone();
    fs::remove_dir(&workspace).unwrap();
    let store = ConversationStore::at(data.path().join("data"));

    let error = store
        .create_turn_with_workspace("user", "assistant", Some(&selected))
        .unwrap_err();

    assert!(error.contains("workspace"), "{error}");
    assert!(store.list().unwrap().is_empty());
    assert!(!data.path().join("data/conversations").exists());
}

#[cfg(unix)]
#[test]
fn workspace_validation_rejects_non_utf8_paths() {
    use std::os::unix::ffi::OsStringExt;

    let data = tempdir().unwrap();
    let path = data
        .path()
        .join(PathBuf::from(std::ffi::OsString::from_vec(vec![
            b'w', 0xff,
        ])));
    fs::create_dir(&path).unwrap();
    let store = ConversationStore::at(data.path().join("data"));

    let error = store
        .create_turn_with_workspace("user", "assistant", Some(&path))
        .unwrap_err();

    assert!(error.contains("valid Unicode"), "{error}");
    assert!(store.list().unwrap().is_empty());
}

#[test]
fn from_user_data_prefers_atlas_data_dir_override() {
    let _guard = env_lock();
    let data = tempdir().unwrap();
    let previous = env::var_os("ATLAS_DATA_DIR");
    env::set_var("ATLAS_DATA_DIR", data.path());

    let result = ConversationStore::from_user_data()
        .and_then(|store| store.create_turn("user", "assistant"));

    match previous {
        Some(value) => env::set_var("ATLAS_DATA_DIR", value),
        None => env::remove_var("ATLAS_DATA_DIR"),
    }
    result.unwrap();
    assert!(data.path().join("conversations.json").is_file());
}

#[test]
fn append_load_list_rename_and_record_title_attempt_update_expected_fields() {
    let data = tempdir().unwrap();
    let store = ConversationStore::at(data.path().to_path_buf());
    let first = store.create_turn("one", "two").unwrap();
    thread::sleep(Duration::from_millis(2));
    let second = store.create_turn("other", "answer").unwrap();
    thread::sleep(Duration::from_millis(2));

    let appended = store.append_turn(&first.id, "three", "four").unwrap();
    assert_eq!(appended.message_count, 4);
    assert!(appended.updated_at > first.updated_at);
    assert_eq!(appended.created_at, first.created_at);

    let loaded = store.load(&first.id).unwrap();
    let roles_and_content = loaded
        .messages
        .iter()
        .map(|message| (&message.role, message.content.as_str()))
        .collect::<Vec<_>>();
    assert_eq!(
        roles_and_content,
        vec![
            (&MessageRole::User, "one"),
            (&MessageRole::Assistant, "two"),
            (&MessageRole::User, "three"),
            (&MessageRole::Assistant, "four"),
        ]
    );

    let listed = store.list().unwrap();
    assert_eq!(listed.len(), 2);
    assert_eq!(listed[0].id, first.id);
    assert_eq!(listed[1].id, second.id);

    let renamed = store.rename(&first.id, "My conversation").unwrap();
    assert_eq!(renamed.title, "My conversation");
    assert!(renamed.title_attempted);
    assert_eq!(renamed.created_at, appended.created_at);
    assert_eq!(renamed.updated_at, appended.updated_at);
    assert_eq!(renamed.message_count, appended.message_count);

    let titled = store
        .record_title_attempt(&first.id, Some("Sanitized title"))
        .unwrap();
    assert_eq!(titled.title, "My conversation");
    assert!(titled.title_attempted);
    assert_eq!(titled.updated_at, renamed.updated_at);

    let unchanged_title = store.record_title_attempt(&second.id, None).unwrap();
    assert_eq!(unchanged_title.title, FALLBACK_TITLE);
    assert!(unchanged_title.title_attempted);
}

#[test]
fn append_assigns_workspace_only_when_missing() {
    let root = tempdir().unwrap();
    let workspace = root.path().join("workspace");
    fs::create_dir(&workspace).unwrap();
    let store = ConversationStore::at(root.path().join("data"));
    let created = store.create_turn("one", "two").unwrap();

    let appended = store
        .append_turn_with_workspace_if_missing(&created.id, "three", "four", Some(&workspace))
        .unwrap();

    assert_eq!(appended.message_count, 4);
    assert_eq!(
        appended.workspace_path,
        Some(workspace.canonicalize().unwrap())
    );
    let loaded = store.load(&created.id).unwrap();
    assert_eq!(loaded.messages.len(), 4);
    assert_eq!(loaded.messages[2].content, "three");
    assert_eq!(loaded.messages[3].content, "four");
}

#[test]
fn append_preserves_an_existing_workspace_over_a_fallback() {
    let root = tempdir().unwrap();
    let assigned = root.path().join("assigned");
    let fallback = root.path().join("fallback");
    fs::create_dir(&assigned).unwrap();
    fs::create_dir(&fallback).unwrap();
    let store = ConversationStore::at(root.path().join("data"));
    let created = store
        .create_turn_with_workspace("one", "two", Some(&assigned))
        .unwrap();

    let appended = store
        .append_turn_with_workspace_if_missing(&created.id, "three", "four", Some(&fallback))
        .unwrap();

    assert_eq!(appended.message_count, 4);
    assert_eq!(
        appended.workspace_path,
        Some(assigned.canonicalize().unwrap())
    );
    assert_eq!(store.load(&created.id).unwrap().messages.len(), 4);
}

#[test]
fn append_preserves_a_workspace_assigned_after_fallback_resolution() {
    let root = tempdir().unwrap();
    let assigned = root.path().join("assigned");
    let fallback = root.path().join("fallback");
    fs::create_dir(&assigned).unwrap();
    fs::create_dir(&fallback).unwrap();
    let store = ConversationStore::at(root.path().join("data"));
    let created = store.create_turn("one", "two").unwrap();

    store.assign_workspace(&created.id, &assigned).unwrap();
    let appended = store
        .append_turn_with_workspace_if_missing(&created.id, "three", "four", Some(&fallback))
        .unwrap();

    assert_eq!(
        appended.workspace_path,
        Some(assigned.canonicalize().unwrap())
    );
    assert_eq!(store.load(&created.id).unwrap().messages.len(), 4);
}

#[test]
fn delete_moves_history_to_trash_and_removes_metadata() {
    let data = tempdir().unwrap();
    let store = ConversationStore::at(data.path().to_path_buf());
    let metadata = store.create_turn("user", "assistant").unwrap();
    let original = data
        .path()
        .join("conversations")
        .join(format!("{}.jsonl", metadata.id));

    let trashed = store.delete(&metadata.id).unwrap();

    assert_eq!(
        trashed,
        data.path()
            .join("trash")
            .join(format!("{}.jsonl", metadata.id))
    );
    assert!(!original.exists());
    assert!(trashed.is_file());
    assert!(store.list().unwrap().is_empty());
    assert_eq!(
        store.load(&metadata.id).unwrap_err(),
        format!("conversation not found: {}", metadata.id)
    );
}

#[test]
fn typed_ids_reject_path_traversal_and_missing_ids_are_explicit() {
    assert!(ConversationId::parse("../conversations.json").is_err());

    let data = tempdir().unwrap();
    let store = ConversationStore::at(data.path().to_path_buf());
    let missing = ConversationId::new();
    let expected = format!("conversation not found: {missing}");

    assert_eq!(store.load(&missing).unwrap_err(), expected);
    assert_eq!(store.append_turn(&missing, "u", "a").unwrap_err(), expected);
    assert_eq!(store.rename(&missing, "title").unwrap_err(), expected);
    assert_eq!(
        store.record_title_attempt(&missing, None).unwrap_err(),
        expected
    );
    assert_eq!(store.delete(&missing).unwrap_err(), expected);
}

#[test]
fn concurrent_appends_write_complete_non_interleaved_lines() {
    let data = tempdir().unwrap();
    let store = Arc::new(ConversationStore::at(data.path().to_path_buf()));
    let metadata = store
        .create_turn("initial user", "initial assistant")
        .unwrap();

    let handles = (0..12)
        .map(|number| {
            let store = Arc::clone(&store);
            let id = metadata.id.clone();
            thread::spawn(move || {
                store
                    .append_turn(
                        &id,
                        &format!("user {number}"),
                        &format!("assistant {number}"),
                    )
                    .unwrap();
            })
        })
        .collect::<Vec<_>>();
    for handle in handles {
        handle.join().unwrap();
    }

    let loaded = store.load(&metadata.id).unwrap();
    assert_eq!(loaded.messages.len(), 26);
    assert_eq!(loaded.metadata.message_count, 26);
    for pair in loaded.messages[2..].chunks_exact(2) {
        assert_eq!(pair[0].role, MessageRole::User);
        assert_eq!(pair[1].role, MessageRole::Assistant);
        assert_eq!(
            pair[0].content.strip_prefix("user "),
            pair[1].content.strip_prefix("assistant ")
        );
    }
}

#[test]
fn append_preserves_a_complete_final_record_without_a_newline() {
    let data = tempdir().unwrap();
    let store = ConversationStore::at(data.path().to_path_buf());
    let metadata = store.create_turn("first user", "first assistant").unwrap();
    let history_path = data
        .path()
        .join("conversations")
        .join(format!("{}.jsonl", metadata.id));
    let mut history = fs::read(&history_path).unwrap();
    assert_eq!(history.pop(), Some(b'\n'));
    fs::write(&history_path, history).unwrap();

    store
        .append_turn(&metadata.id, "second user", "second assistant")
        .unwrap();

    let loaded = store.load(&metadata.id).unwrap();
    assert_eq!(loaded.messages.len(), 4);
    assert_eq!(loaded.messages[1].content, "first assistant");
    assert_eq!(loaded.messages[2].content, "second user");
    assert_eq!(loaded.messages[3].content, "second assistant");
}

#[test]
fn load_waits_for_a_partial_append_to_finish_under_the_store_lock() {
    let data = tempdir().unwrap();
    let store = ConversationStore::at(data.path().to_path_buf());
    let metadata = store.create_turn("first user", "first assistant").unwrap();
    let history_path = data
        .path()
        .join("conversations")
        .join(format!("{}.jsonl", metadata.id));
    let lock = lock_store_exclusively(data.path());
    let message = serde_json::to_string(&ConversationMessage {
        role: MessageRole::User,
        content: "completed after load starts".to_string(),
        created_at: Utc::now(),
    })
    .unwrap();
    let split = message.len() / 2;
    let mut history = fs::OpenOptions::new()
        .append(true)
        .open(&history_path)
        .unwrap();
    history.write_all(&message.as_bytes()[..split]).unwrap();
    history.flush().unwrap();

    let (started_tx, started_rx) = mpsc::channel();
    let (result_tx, result_rx) = mpsc::channel();
    let load_store = store.clone();
    let id = metadata.id.clone();
    let handle = thread::spawn(move || {
        started_tx.send(()).unwrap();
        result_tx.send(load_store.load(&id)).unwrap();
    });
    started_rx.recv().unwrap();

    assert!(matches!(
        result_rx.recv_timeout(Duration::from_millis(100)),
        Err(mpsc::RecvTimeoutError::Timeout)
    ));
    history.write_all(&message.as_bytes()[split..]).unwrap();
    history.write_all(b"\n").unwrap();
    history.flush().unwrap();
    drop(history);
    FileExt::unlock(&lock).unwrap();

    let loaded = result_rx
        .recv_timeout(Duration::from_secs(1))
        .unwrap()
        .unwrap();
    handle.join().unwrap();
    assert_eq!(loaded.messages.len(), 3);
    assert_eq!(loaded.messages[2].content, "completed after load starts");
}

#[test]
fn load_waits_for_an_in_flight_delete_to_restore_or_publish() {
    let data = tempdir().unwrap();
    let store = ConversationStore::at(data.path().to_path_buf());
    let metadata = store.create_turn("user", "assistant").unwrap();
    let history_path = data
        .path()
        .join("conversations")
        .join(format!("{}.jsonl", metadata.id));
    let trash_dir = data.path().join("trash");
    fs::create_dir(&trash_dir).unwrap();
    let trash_path = trash_dir.join(format!("{}.jsonl", metadata.id));
    let lock = lock_store_exclusively(data.path());
    fs::rename(&history_path, &trash_path).unwrap();

    let (started_tx, started_rx) = mpsc::channel();
    let (result_tx, result_rx) = mpsc::channel();
    let load_store = store.clone();
    let id = metadata.id.clone();
    let handle = thread::spawn(move || {
        started_tx.send(()).unwrap();
        result_tx.send(load_store.load(&id)).unwrap();
    });
    started_rx.recv().unwrap();

    assert!(matches!(
        result_rx.recv_timeout(Duration::from_millis(100)),
        Err(mpsc::RecvTimeoutError::Timeout)
    ));
    fs::rename(&trash_path, &history_path).unwrap();
    FileExt::unlock(&lock).unwrap();

    let loaded = result_rx
        .recv_timeout(Duration::from_secs(1))
        .unwrap()
        .unwrap();
    handle.join().unwrap();
    assert_eq!(loaded.metadata.id, metadata.id);
    assert_eq!(loaded.messages.len(), 2);
}

#[test]
fn failed_delete_publication_rolls_back_for_load_and_retry() {
    let data = tempdir().unwrap();
    let store = store_failing_index_publication(data.path(), 2);
    let metadata = store.create_turn("user", "assistant").unwrap();
    let history_path = data
        .path()
        .join("conversations")
        .join(format!("{}.jsonl", metadata.id));
    let trash_path = data
        .path()
        .join("trash")
        .join(format!("{}.jsonl", metadata.id));

    let error = store.delete(&metadata.id).unwrap_err();

    assert!(
        error.contains("injected index publication failure"),
        "{error}"
    );
    assert!(error.contains("deletion rolled back"), "{error}");
    assert!(history_path.is_file());
    assert!(!trash_path.exists());
    assert_eq!(store.load(&metadata.id).unwrap().messages.len(), 2);

    let retried_path = store.delete(&metadata.id).unwrap();
    assert_eq!(retried_path, trash_path);
    assert!(retried_path.is_file());
    assert!(store.list().unwrap().is_empty());
}

#[test]
fn failed_create_publication_preserves_completed_history_with_actionable_error() {
    let data = tempdir().unwrap();
    let store = store_failing_index_publication(data.path(), 1);

    let error = store
        .create_turn("saved user", "saved assistant")
        .unwrap_err();

    assert!(
        error.contains("injected index publication failure"),
        "{error}"
    );
    assert!(
        error.contains("conversation history preserved at"),
        "{error}"
    );
    let history_paths = fs::read_dir(data.path().join("conversations"))
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect::<Vec<_>>();
    assert_eq!(history_paths.len(), 1);
    assert!(
        error.contains(&history_paths[0].display().to_string()),
        "{error}"
    );
    let messages = fs::read_to_string(&history_paths[0])
        .unwrap()
        .lines()
        .map(serde_json::from_str::<ConversationMessage>)
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert_eq!(messages.len(), 2);
    assert_eq!(messages[0].content, "saved user");
    assert_eq!(messages[1].content, "saved assistant");
    assert!(store.list().unwrap().is_empty());
}

#[test]
fn corrupt_index_error_names_index_path() {
    let data = tempdir().unwrap();
    fs::write(data.path().join("conversations.json"), "not json").unwrap();
    let store = ConversationStore::at(data.path().to_path_buf());

    let error = store.list().unwrap_err();

    assert!(error.contains("conversations.json"), "{error}");
}

#[test]
fn malformed_history_error_names_path_and_one_based_line() {
    let data = tempdir().unwrap();
    let store = ConversationStore::at(data.path().to_path_buf());
    let metadata = store.create_turn("user", "assistant").unwrap();
    let history_path = data
        .path()
        .join("conversations")
        .join(format!("{}.jsonl", metadata.id));
    fs::write(
        &history_path,
        format!(
            "{}\nnot json\n",
            serde_json::to_string(&ConversationMessage {
                role: MessageRole::User,
                content: "valid".to_string(),
                created_at: Utc::now(),
            })
            .unwrap()
        ),
    )
    .unwrap();

    let error = store.load(&metadata.id).unwrap_err();

    assert!(
        error.contains(&history_path.display().to_string()),
        "{error}"
    );
    assert!(error.contains("line 2"), "{error}");
}

#[test]
fn mutation_reconciles_stale_metadata_from_valid_history() {
    let data = tempdir().unwrap();
    let store = ConversationStore::at(data.path().to_path_buf());
    let metadata = store.create_turn("one", "two").unwrap();
    let history_path = data
        .path()
        .join("conversations")
        .join(format!("{}.jsonl", metadata.id));
    let recovered_time = Utc::now() + ChronoDuration::seconds(1);
    let recovered_messages = [
        ConversationMessage {
            role: MessageRole::User,
            content: "recovered user".to_string(),
            created_at: recovered_time,
        },
        ConversationMessage {
            role: MessageRole::Assistant,
            content: "recovered assistant".to_string(),
            created_at: recovered_time,
        },
    ];
    let extra = recovered_messages
        .iter()
        .map(serde_json::to_string)
        .collect::<Result<Vec<_>, _>>()
        .unwrap()
        .join("\n");
    fs::OpenOptions::new()
        .append(true)
        .open(&history_path)
        .and_then(|mut file| {
            use std::io::Write;
            writeln!(file, "{extra}")
        })
        .unwrap();
    assert_eq!(
        read_index(data.path())["conversations"][0]["message_count"],
        2
    );

    let reconciled = store.rename(&metadata.id, "Recovered").unwrap();

    assert_eq!(reconciled.message_count, 4);
    assert_eq!(reconciled.updated_at, recovered_time);
    assert_eq!(store.load(&metadata.id).unwrap().metadata, reconciled);
    assert_eq!(
        read_index(data.path())["conversations"][0],
        json!(reconciled)
    );
}
