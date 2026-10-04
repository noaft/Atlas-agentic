mod commands;

use std::path::{Path, PathBuf};

use atlas_agentic::{
    app::load_dotenv,
    conversations::{history_limit_from_env, ConversationStore},
};
use commands::DesktopState;
use tauri::Manager;

fn bundled_agent_path(resource_dir: &Path) -> PathBuf {
    resource_dir.join("agents/atlas.toml")
}

fn create_desktop_state(
    store: ConversationStore,
    history_limit: usize,
    workspace_root: &Path,
    resource_dir: &Path,
) -> DesktopState {
    DesktopState::with_runtime_paths(
        store,
        history_limit,
        workspace_root.to_path_buf(),
        bundled_agent_path(resource_dir),
    )
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    load_dotenv();
    tauri::Builder::default()
        .setup(|app| {
            let store = ConversationStore::from_user_data().map_err(std::io::Error::other)?;
            let history_limit = history_limit_from_env().map_err(std::io::Error::other)?;
            let workspace_root = std::env::current_dir()?;
            let resource_dir = app.path().resource_dir().map_err(std::io::Error::other)?;
            app.manage(create_desktop_state(
                store,
                history_limit,
                &workspace_root,
                &resource_dir,
            ));
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::list_conversations,
            commands::load_conversation,
            commands::rename_conversation,
            commands::delete_conversation,
            commands::ask
        ])
        .run(tauri::generate_context!())
        .expect("failed to run Atlas desktop application");
}

#[cfg(test)]
mod tests {
    use atlas_agentic::conversations::ConversationStore;
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
        let workspace = TempDir::new().unwrap();
        let store = ConversationStore::at(data.path().to_path_buf());

        let _state = create_desktop_state(store, 40, workspace.path(), resources.path());

        assert!(!resources.path().join("agents/atlas.toml").exists());
    }
}
