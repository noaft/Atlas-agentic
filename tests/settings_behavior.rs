use std::{
    collections::{BTreeMap, BTreeSet},
    ffi::OsString,
    fs, io,
    path::PathBuf,
    sync::{
        atomic::{AtomicUsize, Ordering},
        mpsc, Arc,
    },
    thread,
};

use atlas_agentic::{
    permissions::PermissionMode,
    settings::{AtlasSettings, SettingField, SettingsStore, SETTINGS_VERSION},
};
use tempfile::tempdir;

#[test]
fn missing_settings_file_returns_version_one_defaults() {
    let data = tempdir().unwrap();
    let store = SettingsStore::at(data.path().join("settings.json"));

    let settings = store.load().unwrap();

    assert_eq!(settings.version, SETTINGS_VERSION);
    assert_eq!(settings.deepseek.api_key, None);
    assert_eq!(settings.deepseek.base_url, "https://api.deepseek.com");
    assert_eq!(settings.deepseek.model, "deepseek-flash");
    assert_eq!(settings.conversation.history_limit, 40);
    assert_eq!(settings.workspace.default_path, None);
    assert_eq!(settings.permissions.default_mode, PermissionMode::Ask);
    assert_eq!(
        settings
            .resolve_with(|_| None)
            .unwrap()
            .default_permission_mode,
        PermissionMode::Ask
    );
}

#[test]
fn permission_modes_round_trip_and_legacy_settings_default_to_ask() {
    let data = tempdir().unwrap();
    let path = data.path().join("settings.json");
    let store = SettingsStore::at(path.clone());

    for mode in [
        PermissionMode::Ask,
        PermissionMode::Workspace,
        PermissionMode::FullAccess,
    ] {
        let mut settings = AtlasSettings::default();
        settings.permissions.default_mode = mode;
        store.save(&settings).unwrap();
        assert_eq!(store.load().unwrap().permissions.default_mode, mode);
    }

    fs::write(
        &path,
        r#"{"version":1,"deepseek":{"apiKey":null,"baseUrl":"https://api.deepseek.com","model":"deepseek-flash"},"conversation":{"historyLimit":40},"workspace":{"defaultPath":null}}"#,
    )
    .unwrap();
    assert_eq!(
        store.load().unwrap().permissions.default_mode,
        PermissionMode::Ask
    );
}

#[test]
fn round_trip_preserves_api_base_model_history_and_default_workspace() {
    let data = tempdir().unwrap();
    let workspace = data.path().join("workspace");
    fs::create_dir(&workspace).unwrap();
    let store = SettingsStore::at(data.path().join("config/settings.json"));
    let mut settings = AtlasSettings::default();
    settings.deepseek.api_key = Some("local-secret".to_string());
    settings.deepseek.base_url = "https://models.example.test".to_string();
    settings.deepseek.model = "deepseek-coder".to_string();
    settings.conversation.history_limit = 24;
    settings.workspace.default_path = Some(workspace);

    store.save(&settings).unwrap();

    assert_eq!(store.load().unwrap(), settings);
}

#[test]
fn rejects_zero_history_blank_model_blank_base_url_and_unknown_version() {
    let data = tempdir().unwrap();
    let path = data.path().join("settings.json");
    let store = SettingsStore::at(path.clone());

    let mut zero_history = AtlasSettings::default();
    zero_history.conversation.history_limit = 0;
    assert!(store.save(&zero_history).unwrap_err().contains("history"));

    let mut blank_model = AtlasSettings::default();
    blank_model.deepseek.model = "  ".to_string();
    assert!(store.save(&blank_model).unwrap_err().contains("model"));

    let mut blank_url = AtlasSettings::default();
    blank_url.deepseek.base_url = "\t".to_string();
    assert!(store.save(&blank_url).unwrap_err().contains("base URL"));

    fs::write(
        &path,
        r#"{"version":999,"deepseek":{"apiKey":null,"baseUrl":"https://api.deepseek.com","model":"deepseek-flash"},"conversation":{"historyLimit":40},"workspace":{"defaultPath":null}}"#,
    )
    .unwrap();
    assert!(store
        .load()
        .unwrap_err()
        .contains("unsupported settings version: 999"));
}

#[test]
fn failed_publication_preserves_previous_settings() {
    let data = tempdir().unwrap();
    let path = data.path().join("settings.json");
    let publications = Arc::new(AtomicUsize::new(0));
    let store = SettingsStore::at_with_publisher(path, {
        let publications = Arc::clone(&publications);
        move |source, destination| {
            if publications.fetch_add(1, Ordering::SeqCst) == 1 {
                Err(io::Error::other("injected publication failure"))
            } else {
                fs::rename(source, destination)
            }
        }
    });
    let original = AtlasSettings::default();
    store.save(&original).unwrap();
    let mut replacement = original.clone();
    replacement.deepseek.model = "replacement".to_string();

    let error = store.save(&replacement).unwrap_err();

    assert!(error.contains("injected publication failure"), "{error}");
    assert_eq!(store.load().unwrap(), original);
}

#[test]
fn concurrent_writers_leave_one_complete_valid_document() {
    let data = tempdir().unwrap();
    let store = Arc::new(SettingsStore::at(data.path().join("settings.json")));
    let handles = (0..12)
        .map(|number| {
            let store = Arc::clone(&store);
            thread::spawn(move || {
                let mut settings = AtlasSettings::default();
                settings.deepseek.model = format!("model-{number}");
                store.save(&settings).unwrap();
            })
        })
        .collect::<Vec<_>>();

    for handle in handles {
        handle.join().unwrap();
    }

    let saved = store.load().unwrap();
    assert!(saved.deepseek.model.starts_with("model-"));
    let number = saved.deepseek.model[6..].parse::<usize>().unwrap();
    assert!(number < 12);
}

#[test]
fn atomic_updates_preserve_changes_from_overlapping_writers() {
    let data = tempdir().unwrap();
    let store = Arc::new(SettingsStore::at(data.path().join("settings.json")));
    store.save(&AtlasSettings::default()).unwrap();
    let (entered_tx, entered_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let first_store = Arc::clone(&store);
    let first = thread::spawn(move || {
        first_store
            .update(|settings| {
                entered_tx.send(()).unwrap();
                release_rx.recv().unwrap();
                settings.deepseek.model = "selected-model".to_string();
                Ok(())
            })
            .unwrap();
    });
    entered_rx.recv().unwrap();
    let (attempted_tx, attempted_rx) = mpsc::channel();
    let second_store = Arc::clone(&store);
    let second = thread::spawn(move || {
        attempted_tx.send(()).unwrap();
        second_store
            .update(|settings| {
                settings.conversation.history_limit = 17;
                Ok(())
            })
            .unwrap();
    });
    attempted_rx.recv().unwrap();

    release_tx.send(()).unwrap();
    first.join().unwrap();
    second.join().unwrap();

    let saved = store.load().unwrap();
    assert_eq!(saved.deepseek.model, "selected-model");
    assert_eq!(saved.conversation.history_limit, 17);
}

#[cfg(unix)]
#[test]
fn settings_permissions_are_private_with_permissive_umask() {
    use std::os::unix::fs::PermissionsExt;

    let data = tempdir().unwrap();
    let path = data.path().join("atlas/config/settings.json");
    let store = SettingsStore::at(path.clone());
    let previous = unsafe { libc::umask(0) };

    let result = store.save(&AtlasSettings::default());
    unsafe { libc::umask(previous) };
    result.unwrap();

    assert_eq!(
        fs::metadata(path.parent().unwrap())
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o700
    );
    assert_eq!(
        fs::metadata(path).unwrap().permissions().mode() & 0o777,
        0o600
    );
}

#[test]
fn from_user_config_uses_atlas_config_dir_override() {
    static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let _guard = ENV_LOCK.lock().unwrap();
    let data = tempdir().unwrap();
    let previous = std::env::var_os("ATLAS_CONFIG_DIR");
    std::env::set_var("ATLAS_CONFIG_DIR", data.path());

    let result = SettingsStore::from_user_config().and_then(|store| {
        store.save(&AtlasSettings::default())?;
        Ok(())
    });

    match previous {
        Some(value) => std::env::set_var("ATLAS_CONFIG_DIR", value),
        None => std::env::remove_var("ATLAS_CONFIG_DIR"),
    }
    result.unwrap();
    assert!(PathBuf::from(data.path()).join("settings.json").is_file());
}

#[test]
fn environment_overrides_only_supported_fields_and_records_their_sources() {
    let workspace = PathBuf::from("/saved/workspace");
    let mut saved = AtlasSettings::default();
    saved.deepseek.api_key = Some("saved-key".to_string());
    saved.deepseek.base_url = "https://saved.example.test".to_string();
    saved.deepseek.model = "saved-model".to_string();
    saved.conversation.history_limit = 17;
    saved.workspace.default_path = Some(workspace.clone());
    let environment = BTreeMap::from([
        ("DEEPSEEK_API_KEY", OsString::from("environment-key")),
        (
            "DEEPSEEK_BASE_URL",
            OsString::from("https://environment.example.test"),
        ),
        ("DEEPSEEK_MODEL", OsString::from("environment-model")),
        ("ATLAS_HISTORY_MESSAGES", OsString::from("9")),
    ]);

    let effective = saved
        .resolve_with(|name| environment.get(name).cloned())
        .unwrap();

    assert_eq!(effective.api_key.as_deref(), Some("environment-key"));
    assert_eq!(effective.base_url, "https://environment.example.test");
    assert_eq!(effective.model, "environment-model");
    assert_eq!(effective.history_limit, 9);
    assert_eq!(effective.default_workspace, Some(workspace));
    assert_eq!(
        effective.environment_overrides,
        BTreeSet::from([
            SettingField::ApiKey,
            SettingField::BaseUrl,
            SettingField::Model,
            SettingField::HistoryLimit,
        ])
    );
}

#[test]
fn invalid_environment_history_limit_keeps_existing_error_message() {
    let error = AtlasSettings::default()
        .resolve_with(|name| (name == "ATLAS_HISTORY_MESSAGES").then(|| OsString::from("0")))
        .unwrap_err();

    assert_eq!(error, "ATLAS_HISTORY_MESSAGES must be a positive integer");
}

#[test]
fn resolving_environment_overrides_never_persists_them() {
    let data = tempdir().unwrap();
    let store = SettingsStore::at(data.path().join("settings.json"));
    let mut saved = AtlasSettings::default();
    saved.deepseek.api_key = Some("saved-key".to_string());
    store.save(&saved).unwrap();

    let effective = store
        .load()
        .unwrap()
        .resolve_with(|name| {
            (name == "DEEPSEEK_API_KEY").then(|| OsString::from("environment-key"))
        })
        .unwrap();

    assert_eq!(effective.api_key.as_deref(), Some("environment-key"));
    assert_eq!(store.load().unwrap(), saved);
}

#[test]
fn effective_settings_debug_output_redacts_the_api_key() {
    let mut saved = AtlasSettings::default();
    saved.deepseek.api_key = Some("never-print-this-secret".to_string());

    let saved_debug = format!("{saved:?}");
    let effective_debug = format!("{:?}", saved.resolve_with(|_| None).unwrap());

    assert!(
        saved_debug.contains("api_key_configured: true"),
        "{saved_debug}"
    );
    assert!(
        effective_debug.contains("api_key_configured: true"),
        "{effective_debug}"
    );
    assert!(
        !saved_debug.contains("never-print-this-secret"),
        "{saved_debug}"
    );
    assert!(
        !effective_debug.contains("never-print-this-secret"),
        "{effective_debug}"
    );
}
