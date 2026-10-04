use std::{
    env, fs, io,
    io::Write,
    sync::{
        atomic::{AtomicUsize, Ordering},
        mpsc, Arc, Mutex, OnceLock,
    },
    thread,
    time::Duration,
};

use atlas_agentic::conversations::{
    ConversationId, ConversationMessage, ConversationStore, MessageRole,
};
use chrono::{Duration as ChronoDuration, Utc};
use fs2::FileExt;
use serde_json::{json, Value};
use tempfile::tempdir;

const FALLBACK_TITLE: &str = "Untitled conversation";

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
