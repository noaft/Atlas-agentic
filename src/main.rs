use std::io::Write;

use atlas_agentic::{
    agents::{AgentRegistry, LocalAgent, LocalAgentDefinition},
    conversations::{
        history_limit_from_env, ConversationId, ConversationService, ConversationStore,
    },
    handle_command,
    models::provider_from_definition,
    render_conversation, render_conversation_metadata, render_conversation_summary, render_help,
    tools::{CreateFileTool, ListDirTool, PwdTool, ReadFileTool, ToolRegistry},
    AppResult, Runtime,
};

#[cfg(unix)]
use atlas_agentic::tools::ExecCommandTool;

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
    let history_limit = history_limit_from_env()?;
    let store = ConversationStore::from_user_data()?;
    let runtime = build_runtime()?;
    let service = ConversationService::new(&runtime, &store, history_limit);
    let mut stdout = std::io::stdout();
    let result = service.ask(prompt, id, &mut |delta| {
        print!("{delta}");
        let _ = stdout.flush();
    })?;
    println!("\n{}", render_conversation_metadata(&result.metadata));
    Ok(())
}

fn build_runtime() -> Result<Runtime, String> {
    let definition = LocalAgentDefinition::from_file("agents/atlas.toml")?;
    let provider = provider_from_definition(&definition)?;
    let tools = ToolRegistry::new(vec![
        Box::new(PwdTool::new(".")),
        Box::new(ListDirTool::new(".")),
        Box::new(ReadFileTool::new(".")),
        Box::new(CreateFileTool::new(".")),
    ]);
    #[cfg(unix)]
    let tools = {
        let mut tools = tools;
        tools.register(Box::new(ExecCommandTool::new(".")));
        tools
    };
    Ok(
        Runtime::new(AgentRegistry::new(vec![Box::new(LocalAgent::new(
            definition, provider,
        ))]))
        .with_tools(tools),
    )
}

fn load_dotenv() {
    if std::env::var_os("ATLAS_SKIP_DOTENV").is_none() {
        let _ = dotenvy::dotenv();
    }
}
