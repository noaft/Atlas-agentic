mod commands;
mod mcp_commands;
mod provider_settings;
mod requests;
mod skill_commands;

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
    atlas_agentic::diagnostics::init("desktop");
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
            skill_commands::list_skills,
            skill_commands::read_skill,
            skill_commands::preview_skill_import,
            skill_commands::install_skill,
            skill_commands::set_skill_enabled,
            mcp_commands::list_mcp_connections,
            mcp_commands::connect_mcp_server,
            mcp_commands::disconnect_mcp_server,
            mcp_commands::cancel_mcp_connection,
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
            commands::list_tasks,
            commands::run_task,
            commands::ask
        ])
        .run(tauri::generate_context!())
        .unwrap_or_else(|error| {
            atlas_agentic::diagnostics::event(
                "ERROR",
                "application.failed",
                serde_json::json!({"error": error.to_string()}),
            );
            panic!("failed to run Atlas desktop application: {error}");
        });
    atlas_agentic::diagnostics::event("INFO", "application.stopped", serde_json::json!({}));
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
