mod commands;

use atlas_agentic::conversations::ConversationStore;
use commands::DesktopState;
use tauri::Manager;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .setup(|app| {
            let store = ConversationStore::from_user_data().map_err(std::io::Error::other)?;
            app.manage(DesktopState::new(store));
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::list_conversations,
            commands::load_conversation,
            commands::rename_conversation,
            commands::delete_conversation
        ])
        .run(tauri::generate_context!())
        .expect("failed to run Atlas desktop application");
}
