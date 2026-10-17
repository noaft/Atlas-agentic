mod commands;
mod requests;

use std::path::{Path, PathBuf};

use atlas_agentic::{app::load_dotenv, conversations::ConversationStore, settings::SettingsStore};
use commands::DesktopState;
use tauri::Manager;

fn bundled_agent_path(resource_dir: &Path) -> PathBuf {
    resource_dir.join("agents/atlas.toml")
}

fn create_desktop_state(
    store: ConversationStore,
    settings_store: SettingsStore,
    resource_dir: &Path,
) -> DesktopState {
    DesktopState::with_runtime_paths(store, settings_store, bundled_agent_path(resource_dir))
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    load_dotenv();
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            let store = ConversationStore::from_user_data().map_err(std::io::Error::other)?;
            let settings_store =
                SettingsStore::from_user_config().map_err(std::io::Error::other)?;
            let resource_dir = app.path().resource_dir().map_err(std::io::Error::other)?;
            app.manage(create_desktop_state(store, settings_store, &resource_dir));
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::list_conversations,
            commands::load_conversation,
            commands::rename_conversation,
            commands::delete_conversation,
            commands::load_settings,
            commands::save_settings,
            commands::list_models,
            commands::set_model,
            commands::validate_workspace,
            commands::assign_conversation_workspace,
            commands::set_conversation_permission_mode,
            commands::resolve_tool_approval,
            commands::cancel_tool_approvals,
            commands::cancel_compaction,
            commands::compact_conversation,
            commands::ask
        ])
        .run(tauri::generate_context!())
        .expect("failed to run Atlas desktop application");
}

#[cfg(test)]
mod tests {
    use atlas_agentic::{conversations::ConversationStore, settings::SettingsStore};
    use tempfile::TempDir;

    use super::{bundled_agent_path, create_desktop_state};

    #[test]
    fn bundled_agent_path_uses_the_tauri_resource_directory() {
        let resources = TempDir::new().unwrap();

        assert_eq!(
            bundled_agent_path(resources.path()),
            resources.path().join("agents/atlas.toml")
        );
    }

    #[test]
    fn desktop_state_setup_is_lazy_about_agent_and_provider_configuration() {
        let data = TempDir::new().unwrap();
        let resources = TempDir::new().unwrap();
        let config = TempDir::new().unwrap();
        let store = ConversationStore::at(data.path().to_path_buf());
        let settings = SettingsStore::at(config.path().join("settings.json"));

        let _state = create_desktop_state(store, settings, resources.path());

        assert!(!resources.path().join("agents/atlas.toml").exists());
    }
}
