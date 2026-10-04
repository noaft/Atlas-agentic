use std::{io::Write, path::Path};

use atlas_agentic::{
    app::{build_runtime_with_settings, load_dotenv},
    conversations::{ConversationId, ConversationService, ConversationStore},
    handle_command, render_conversation, render_conversation_metadata, render_conversation_summary,
    render_help,
    settings::SettingsStore,
    AppResult,
};

fn main() {
    load_dotenv();
    if let Err(error) = handle_command(std::env::args().skip(1)).and_then(run_command) {
        eprintln!("Error: {error}");
        std::process::exit(1);
    }
}

fn run_command(command: AppResult) -> Result<(), String> {
    match command {
        AppResult::Help => println!("{}", render_help()),
        AppResult::Ask {
            prompt,
            conversation_id,
        } => run_ask(&prompt, conversation_id.as_ref())?,
        AppResult::ConversationList => {
            let conversations = ConversationStore::from_user_data()?.list()?;
            if conversations.is_empty() {
                println!("No conversations.");
            } else {
                println!(
                    "{}",
                    conversations
                        .iter()
                        .map(render_conversation_summary)
                        .collect::<Vec<_>>()
                        .join("\n\n")
                );
            }
        }
        AppResult::ConversationShow { id } => {
            println!(
                "{}",
                render_conversation(&ConversationStore::from_user_data()?.load(&id)?)
            );
        }
        AppResult::ConversationRename { id, title } => {
            let metadata = ConversationStore::from_user_data()?.rename(&id, &title)?;
            println!("Renamed conversation: {} | {}", metadata.id, metadata.title);
        }
        AppResult::ConversationDelete { id } => {
            let path = ConversationStore::from_user_data()?.delete(&id)?;
            println!(
                "Deleted conversation: {id}\nHistory recoverable at: {}",
                path.display()
            );
        }
    }
    Ok(())
}

fn run_ask(prompt: &str, id: Option<&ConversationId>) -> Result<(), String> {
    let settings = SettingsStore::from_user_config()?
        .load()?
        .resolve_from_env()?;
    let store = ConversationStore::from_user_data()?;
    let workspace = std::env::current_dir()
        .map_err(|error| format!("failed to resolve current directory: {error}"))?;
    let runtime =
        build_runtime_with_settings(&workspace, Path::new("agents/atlas.toml"), &settings)?;
    let service = ConversationService::new(&runtime, &store, settings.history_limit);
    let mut stdout = std::io::stdout();
    let result = service.ask(prompt, id, &mut |delta| {
        print!("{delta}");
        let _ = stdout.flush();
    })?;
    println!("\n{}", render_conversation_metadata(&result.metadata));
    Ok(())
}
