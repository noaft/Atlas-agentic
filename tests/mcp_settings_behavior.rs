use atlas_agentic::mcp::{McpConfig, McpConfigStore};
use std::fs;

fn config(text: &str) -> atlas_agentic::mcp::ServerConfig {
    McpConfig::from_toml(&format!("[servers.example]\n{text}"))
        .unwrap()
        .servers
        .remove("example")
        .unwrap()
}

#[test]
fn missing_settings_reads_are_side_effect_free_and_add_publishes_valid_config() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("new/mcp.toml");
    let store = McpConfigStore::new(path.clone()).unwrap();
    let before = store.load().unwrap();
    assert!(before.config.servers.is_empty());
    assert!(!path.parent().unwrap().exists());
    let after = store
        .add(
            &before.fingerprint,
            "local",
            config("command = 'not-installed'"),
        )
        .unwrap();
    assert!(after.config.servers.contains_key("local"));
    assert_ne!(before.fingerprint, after.fingerprint);
    assert_eq!(
        McpConfig::load(&path).unwrap().servers["local"].command,
        "not-installed"
    );
}

#[test]
fn addition_preserves_comments_and_all_original_bytes_and_never_overwrites() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("mcp.toml");
    let original = "# personal comment\n[servers.existing]\ncommand = 'keep-me' # inline comment\n";
    fs::write(&path, original).unwrap();
    let store = McpConfigStore::new(path.clone()).unwrap();
    let first = store.load().unwrap();
    let after = store
        .add(
            &first.fingerprint,
            "remote",
            config(
                "url='https://example.com/mcp'\n[servers.example.oauth]\nclient_id='public-client'",
            ),
        )
        .unwrap();
    let text = fs::read_to_string(&path).unwrap();
    assert!(text.starts_with(original));
    assert_eq!(after.config.servers["existing"].command, "keep-me");
    assert!(after.config.servers["remote"].oauth.is_some());
    assert!(store
        .add(
            &after.fingerprint,
            "existing",
            config("command='replacement'")
        )
        .is_err());
    assert_eq!(fs::read_to_string(&path).unwrap(), text);
    assert!(store
        .add(&first.fingerprint, "stale", config("command='stale'"))
        .is_err());
    assert_eq!(fs::read_to_string(&path).unwrap(), text);
}

#[test]
fn malformed_unsupported_and_oversized_documents_stay_untouched() {
    let root = tempfile::tempdir().unwrap();
    for content in [
        "# secret-value\n[broken".to_string(),
        "unknown = 'secret-value'".into(),
        "x".repeat(1024 * 1024 + 1),
    ] {
        let path = root.path().join("mcp.toml");
        fs::write(&path, &content).unwrap();
        let store = McpConfigStore::new(path.clone()).unwrap();
        let error = store.load().err().unwrap();
        assert!(!error.contains("secret-value"));
        assert!(store
            .add("invalid", "new", config("command='new'"))
            .is_err());
        assert_eq!(fs::read_to_string(&path).unwrap(), content);
    }
}

#[test]
fn fingerprints_bind_config_root_and_invalid_server_additions_do_not_publish() {
    let root = tempfile::tempdir().unwrap();
    let a = McpConfigStore::new(root.path().join("a/mcp.toml")).unwrap();
    let b = McpConfigStore::new(root.path().join("b/mcp.toml")).unwrap();
    let version = a.load().unwrap().fingerprint;
    assert_ne!(version, b.load().unwrap().fingerprint);
    assert!(b.add(&version, "new", config("command='new'")).is_err());
    let before = a.load().unwrap();
    for name in ["", "bad name", "../escape", "name.with.dot"] {
        assert!(a
            .add(&before.fingerprint, name, config("command='new'"))
            .is_err());
    }
    assert!(!a.load().unwrap().config.servers.contains_key("new"));
}

#[cfg(unix)]
#[test]
fn private_files_and_symlink_leaves_are_enforced() {
    use std::os::unix::{fs::symlink, fs::PermissionsExt};
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("private/mcp.toml");
    let store = McpConfigStore::new(path.clone()).unwrap();
    store
        .add(
            &store.load().unwrap().fingerprint,
            "local",
            config("command='new'"),
        )
        .unwrap();
    assert_eq!(
        fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o600
    );
    assert_eq!(
        fs::metadata(path.parent().unwrap())
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o700
    );
    let link = root.path().join("linked.toml");
    symlink(&path, &link).unwrap();
    let linked = McpConfigStore::new(link).unwrap();
    assert!(linked.load().is_err());
    let before = fs::read(&path).unwrap();
    assert!(linked
        .add("anything", "new", config("command='new'"))
        .is_err());
    assert_eq!(fs::read(&path).unwrap(), before);
}

#[test]
fn concurrent_additions_cannot_overwrite_each_other() {
    use std::sync::{Arc, Barrier};
    let root = tempfile::tempdir().unwrap();
    let store = McpConfigStore::new(root.path().join("mcp.toml")).unwrap();
    let fingerprint = store.load().unwrap().fingerprint;
    let barrier = Arc::new(Barrier::new(2));
    let writers: Vec<_> = ["first", "second"]
        .into_iter()
        .map(|name| {
            let (store, fingerprint, barrier) =
                (store.clone(), fingerprint.clone(), barrier.clone());
            std::thread::spawn(move || {
                barrier.wait();
                store.add(&fingerprint, name, config("command='unused'"))
            })
        })
        .collect();
    let results: Vec<_> = writers
        .into_iter()
        .map(|thread| thread.join().unwrap())
        .collect();
    assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
    assert_eq!(store.load().unwrap().config.servers.len(), 1);
}

#[cfg(unix)]
#[test]
fn symlink_lock_and_fifo_configuration_are_rejected_without_blocking() {
    use std::{
        ffi::CString,
        os::unix::{ffi::OsStrExt, fs::symlink},
    };
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("mcp.toml");
    let store = McpConfigStore::new(path.clone()).unwrap();
    let snapshot = store.load().unwrap();
    let target = root.path().join("target");
    fs::write(&target, "keep").unwrap();
    symlink(&target, root.path().join("mcp.toml.lock")).unwrap();
    assert!(store
        .add(&snapshot.fingerprint, "new", config("command='unused'"))
        .is_err());
    assert_eq!(fs::read_to_string(&target).unwrap(), "keep");
    assert!(!path.exists());
    let fifo = CString::new(path.as_os_str().as_bytes()).unwrap();
    assert_eq!(unsafe { libc::mkfifo(fifo.as_ptr(), 0o600) }, 0);
    assert!(store.load().is_err());
}
