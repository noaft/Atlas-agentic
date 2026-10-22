use atlas_agentic::settings::{AtlasSettings, SettingsStore};
use serde_json::json;
use std::{ffi::OsString, fs};

#[test]
fn selected_provider_resolves_only_its_profile_and_environment() {
    let mut value = serde_json::to_value(AtlasSettings::default()).unwrap();
    value["selectedProvider"] = json!("openai");
    value["providers"] = json!({"openai": {"apiKey":"saved-openai", "baseUrl":"https://example.test/v1", "model":"my-model"}});
    let settings: AtlasSettings = serde_json::from_value(value).unwrap();
    let effective = settings
        .resolve_with(|name| (name == "DEEPSEEK_API_KEY").then(|| OsString::from("wrong-key")))
        .unwrap();
    assert_eq!(effective.api_key.as_deref(), Some("saved-openai"));
    assert_eq!(effective.model, "my-model");
}

#[test]
fn migration_is_in_memory_until_save_and_rejects_future_versions() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("settings.json");
    let store = SettingsStore::at(path.clone());
    let mut legacy = serde_json::to_value(AtlasSettings::default()).unwrap();
    legacy["version"] = json!(1);
    let bytes = serde_json::to_vec(&legacy).unwrap();
    fs::write(&path, &bytes).unwrap();
    let loaded = store.load().unwrap();
    assert_eq!(loaded.version, 2);
    assert_eq!(fs::read(&path).unwrap(), bytes);
    store.save(&loaded).unwrap();
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&fs::read(&path).unwrap()).unwrap()["version"],
        2
    );
    legacy["version"] = json!(999);
    let bytes = serde_json::to_vec(&legacy).unwrap();
    fs::write(&path, &bytes).unwrap();
    assert!(store.load().is_err());
    assert_eq!(fs::read(&path).unwrap(), bytes);
}

#[test]
fn provider_urls_cannot_embed_credentials_or_query_secrets() {
    for url in [
        "https://user:secret@example.test",
        "https://example.test?key=secret",
        "file:///tmp/models",
        "https://example.test/#fragment",
    ] {
        let mut value = serde_json::to_value(AtlasSettings::default()).unwrap();
        value["selectedProvider"] = json!("openai");
        value["providers"] = json!({"openai": {"baseUrl":url,"model":"test"}});
        let settings: AtlasSettings = serde_json::from_value(value).unwrap();
        assert!(settings.validate().is_err(), "accepted {url}");
    }
}

#[test]
fn legacy_limit_migration_does_not_retarget_limits_when_endpoint_changes() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("settings.json");
    let mut legacy = serde_json::to_value(AtlasSettings::default()).unwrap();
    legacy["version"] = json!(1);
    legacy["conversation"]["modelLimits"] =
        json!({"deepseek-flash":{"contextWindow":6000,"maxOutputTokens":1000}});
    fs::write(&path, legacy.to_string()).unwrap();
    let mut loaded = SettingsStore::at(path).load().unwrap();
    assert_eq!(loaded.resolve_with(|_| None).unwrap().model_limits.len(), 1);
    loaded.deepseek.base_url = "https://another.test/v1".into();
    assert!(loaded
        .resolve_with(|_| None)
        .unwrap()
        .model_limits
        .is_empty());
}

#[test]
fn native_profiles_can_use_descriptor_endpoint_defaults() {
    let mut value = serde_json::to_value(AtlasSettings::default()).unwrap();
    value["selectedProvider"] = json!("openai");
    value["providers"] = json!({"openai":{"apiKey":"fake","model":"alias"}});
    let settings: AtlasSettings = serde_json::from_value(value).unwrap();
    assert_eq!(
        settings.resolve_with(|_| None).unwrap().base_url,
        "https://api.openai.com/v1"
    );
}
