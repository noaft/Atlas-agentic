#![cfg(unix)]

use std::{
    env, fs,
    os::unix::fs::PermissionsExt,
    process::Command,
    sync::{Arc, Mutex},
};

use atlas_agentic::conversations::{ActivityTurn, ConversationStore};

#[test]
fn storage_permissions_are_private_with_permissive_umask() {
    const CHILD: &str = "ATLAS_TEST_PRIVATE_STORAGE_PERMISSIONS";
    if env::var_os(CHILD).is_none() {
        let output = Command::new(env::current_exe().unwrap())
            .args([
                "--exact",
                "storage_permissions_are_private_with_permissive_umask",
                "--nocapture",
            ])
            .env(CHILD, "1")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "child failed:\n{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        return;
    }

    // SAFETY: umask takes a mode value only. It affects this subprocess, whose
    // command runs exactly this test, so other tests cannot observe the change.
    unsafe { libc::umask(0) };
    let data = tempfile::tempdir().unwrap();
    for existing_root in [false, true] {
        let root = if existing_root {
            let root = data.path().join("existing-root");
            fs::create_dir(&root).unwrap();
            assert_mode(&root, 0o777);
            root
        } else {
            data.path().join("new-parent/atlas")
        };
        let temporary_modes = Arc::new(Mutex::new(Vec::new()));
        let captured_modes = Arc::clone(&temporary_modes);
        let store =
            ConversationStore::at_with_index_publisher(root.clone(), move |source, destination| {
                captured_modes
                    .lock()
                    .unwrap()
                    .push(fs::metadata(source)?.permissions().mode() & 0o777);
                fs::rename(source, destination)
            });
        let metadata = store
            .create_turn_with_activity(
                "private input",
                "private answer",
                None,
                None,
                ActivityTurn {
                    version: 1,
                    turn_id: "private-turn".to_string(),
                    assistant_message_index: usize::MAX,
                    duration_ms: None,
                    usage: None,
                    records: Vec::new(),
                },
            )
            .unwrap();
        store
            .append_turn(&metadata.id, "next input", "next answer")
            .unwrap();
        assert_mode(&root, if existing_root { 0o777 } else { 0o700 });
        if !existing_root {
            assert_mode(root.parent().unwrap(), 0o700);
        }
        assert_mode(&root.join("conversations"), 0o700);
        assert_mode(&root.join("activities"), 0o700);
        assert_mode(&root.join("conversations.json"), 0o600);
        assert_mode(&root.join("conversations.lock"), 0o600);
        assert_mode(
            &root
                .join("conversations")
                .join(format!("{}.jsonl", metadata.id)),
            0o600,
        );
        assert_mode(
            &root
                .join("activities")
                .join(format!("{}.jsonl", metadata.id)),
            0o600,
        );
        let trash_path = store.delete(&metadata.id).unwrap();
        assert_mode(&root.join("trash"), 0o700);
        assert_mode(&trash_path, 0o600);
        assert_mode(&root.join("conversations.json"), 0o600);
        assert_eq!(*temporary_modes.lock().unwrap(), vec![0o600; 3]);
    }
}

fn assert_mode(path: &std::path::Path, expected: u32) {
    assert_eq!(
        fs::metadata(path).unwrap().permissions().mode() & 0o777,
        expected,
        "permissions for {}",
        path.display()
    );
}

#[test]
fn failed_partial_history_append_preserves_existing_conversation() {
    const CHILD: &str = "ATLAS_TEST_PARTIAL_HISTORY_APPEND";
    if env::var_os(CHILD).is_none() {
        let output = Command::new(env::current_exe().unwrap())
            .args([
                "--exact",
                "failed_partial_history_append_preserves_existing_conversation",
                "--nocapture",
            ])
            .env(CHILD, "1")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "child failed:\n{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        return;
    }

    for trailing_newline in [true, false] {
        for input_size in [32, 32768] {
            let data = tempfile::tempdir().unwrap();
            let store = ConversationStore::at(data.path().join("data"));
            let metadata = store.create_turn("saved input", "saved answer").unwrap();
            let history_path = data
                .path()
                .join("data/conversations")
                .join(format!("{}.jsonl", metadata.id));
            let mut original_history = fs::read(&history_path).unwrap();
            if !trailing_newline {
                assert_eq!(original_history.pop(), Some(b'\n'));
                fs::write(&history_path, &original_history).unwrap();
            }
            let original = store.load(&metadata.id).unwrap();
            let index_path = data.path().join("data/conversations.json");
            let original_index = fs::read(&index_path).unwrap();

            // Only this subprocess changes resource limits or signal handling. The
            // kernel accepts 16 appended bytes, then rejects the remaining JSON.
            let mut old_limit = libc::rlimit {
                rlim_cur: 0,
                rlim_max: 0,
            };
            // SAFETY: old_limit is writable and all signal/limit values are valid.
            let old_signal = unsafe {
                assert_eq!(libc::getrlimit(libc::RLIMIT_FSIZE, &mut old_limit), 0);
                let limit = libc::rlimit {
                    rlim_cur: (original_history.len() + 16) as libc::rlim_t,
                    rlim_max: old_limit.rlim_max,
                };
                assert_eq!(libc::setrlimit(libc::RLIMIT_FSIZE, &limit), 0);
                let old_signal = libc::signal(libc::SIGXFSZ, libc::SIG_IGN);
                assert_ne!(old_signal, libc::SIG_ERR);
                old_signal
            };
            let result = store.append_turn(&metadata.id, &"x".repeat(input_size), "new answer");
            // SAFETY: restore the valid values obtained above before assertions.
            unsafe {
                assert_eq!(libc::setrlimit(libc::RLIMIT_FSIZE, &old_limit), 0);
                assert_ne!(libc::signal(libc::SIGXFSZ, old_signal), libc::SIG_ERR);
            }

            assert!(
                result.is_err(),
                "partial history write must return an error"
            );
            assert_eq!(store.load(&metadata.id).unwrap(), original);
            assert_eq!(fs::read(&history_path).unwrap(), original_history);
            assert_eq!(fs::read(&index_path).unwrap(), original_index);
        }
    }
}
