mod commands;

use atlas_agentic::conversations::{history_limit_from_env, ConversationStore};
use commands::DesktopState;
use tauri::Manager;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .setup(|app| {
            let store = ConversationStore::from_user_data().map_err(std::io::Error::other)?;
            let history_limit = history_limit_from_env().map_err(std::io::Error::other)?;
            let workspace_root = std::env::current_dir()?;
            let agent_file = workspace_root.join("agents/atlas.toml");
            app.manage(DesktopState::with_runtime_paths(
                store,
                history_limit,
                workspace_root,
                agent_file,
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
