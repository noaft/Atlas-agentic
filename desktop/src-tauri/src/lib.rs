mod commands;
mod mcp_commands;
mod mcp_inspection;
mod process_commands;
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
    let app = tauri::Builder::default()
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
            skill_commands::create_skill,
            skill_commands::update_skill_document,
            skill_commands::preview_local_skills,
            skill_commands::install_local_skill,
            skill_commands::list_skills,
            skill_commands::read_skill,
            skill_commands::preview_skill_import,
            skill_commands::install_skill,
            skill_commands::install_skills,
            skill_commands::search_skills,
            skill_commands::set_skill_enabled,
            mcp_commands::load_mcp_settings,
            mcp_commands::add_mcp_server,
            mcp_inspection::inspect_mcp_server,
            mcp_inspection::resolve_mcp_inspection_approval,
            mcp_inspection::cancel_mcp_inspection,
            mcp_commands::list_mcp_connections,
            mcp_commands::connect_mcp_server,
            mcp_commands::disconnect_mcp_server,
            mcp_commands::cancel_mcp_connection,
            process_commands::initialize_process_client,
            process_commands::open_process_scope,
            process_commands::list_processes,
            process_commands::release_process_scope,
            process_commands::read_process_output,
            process_commands::stop_process,
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
        .build(tauri::generate_context!())
        .unwrap_or_else(|error| {
            atlas_agentic::diagnostics::event(
                "ERROR",
                "application.failed",
                serde_json::json!({"error": error.to_string()}),
            );
            panic!("failed to run Atlas desktop application: {error}");
        });
    let shutdown = std::sync::Arc::new(std::sync::atomic::AtomicU8::new(0));
    app.run(move |handle, event| {
        if let tauri::RunEvent::ExitRequested { api, code, .. } = event {
            use std::sync::atomic::Ordering;
            if shutdown.load(Ordering::SeqCst) == 2 {
                return;
            }
            api.prevent_exit();
            if shutdown
                .compare_exchange(0, 1, Ordering::SeqCst, Ordering::SeqCst)
                .is_ok()
            {
                let state = handle.state::<DesktopState>().inner().clone();
                let handle = handle.clone();
                let shutdown = shutdown.clone();
                tauri::async_runtime::spawn_blocking(move || {
                    if state.shutdown_processes().is_err() {
                        atlas_agentic::diagnostics::event(
                            "WARN",
                            "process.shutdown.incomplete",
                            serde_json::json!({}),
                        );
                    }
                    shutdown.store(2, Ordering::SeqCst);
                    handle.exit(code.unwrap_or(0));
                });
            }
        }
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
