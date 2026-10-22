use std::{
    io::{BufRead, BufReader, Write},
    path::Path,
    sync::Mutex,
};

use atlas_agentic::{
    app::{build_runtime_with_settings, load_dotenv, resolve_settings_for_agent},
    conversations::{ConversationId, ConversationService, ConversationStore},
    handle_command,
    permissions::{AuthorizationDecision, AuthorizationRequest, ToolAuthorizer},
    render_conversation, render_conversation_metadata, render_conversation_summary, render_help,
    settings::SettingsStore,
    tasks::{TaskOutcome, TaskRegistry, TaskRequest, COMPACT_TASK_ID},
    AppResult,
};

fn main() {
    load_dotenv();
    atlas_agentic::diagnostics::init("cli");
    if let Err(error) =
        atlas_agentic::diagnostics::run("cli.command", serde_json::json!({}), || {
            handle_command(std::env::args().skip(1)).and_then(run_command)
        })
    {
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
        AppResult::ConversationCompact { id } => {
            let effective = resolve_settings_for_agent(
                &SettingsStore::from_user_config()?.load()?,
                Path::new("agents/atlas.toml"),
            )?;
            let store = ConversationStore::from_user_data()?;
            let workspace = std::env::current_dir().map_err(|e| e.to_string())?;
            let runtime = build_runtime_with_settings(
                &workspace,
                Path::new("agents/atlas.toml"),
                &effective,
            )?;
            let service = ConversationService::new(&runtime, &store, effective.history_limit);
            let result = TaskRegistry::builtins().run(
                TaskRequest {
                    task_id: COMPACT_TASK_ID.into(),
                    arguments: serde_json::json!({}),
                    conversation_id: Some(id),
                },
                &service,
                None,
                &mut |progress| {
                    eprintln!(
                        "Compacting context: {}/{}",
                        progress.completed_blocks, progress.total_blocks
                    )
                },
            )?;
            let TaskOutcome::Compaction(result) = result.outcome;
            println!("{}", result.message);
        }
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
    let settings = resolve_settings_for_agent(
        &SettingsStore::from_user_config()?.load()?,
        Path::new("agents/atlas.toml"),
    )?;
    let store = ConversationStore::from_user_data()?;
    let workspace = std::env::current_dir()
        .map_err(|error| format!("failed to resolve current directory: {error}"))?;
    let runtime =
        build_runtime_with_settings(&workspace, Path::new("agents/atlas.toml"), &settings)?;
    let service = ConversationService::new(&runtime, &store, settings.history_limit);
    let authorizer = TerminalAuthorizer::new();
    let mut stdout = std::io::stdout();
    let result = service.ask_with_permission_authorizer(
        prompt,
        id,
        settings.default_permission_mode,
        &authorizer,
        &mut |delta| {
            print!("{delta}");
            let _ = stdout.flush();
        },
        &mut |event| match event {
            atlas_agentic::permissions::ToolEvent::ContextStatus { status }
                if !status.available =>
            {
                eprintln!("Context: {}", status.message)
            }
            atlas_agentic::permissions::ToolEvent::Compaction { progress } => eprintln!(
                "Compacting context: {}/{}",
                progress.completed_blocks, progress.total_blocks
            ),
            _ => {}
        },
    )?;
    println!("\n{}", render_conversation_metadata(&result.metadata));
    Ok(())
}

struct TerminalAuthorizer {
    input: Mutex<Box<dyn BufRead + Send>>,
    output: Mutex<Box<dyn Write + Send>>,
}

impl TerminalAuthorizer {
    fn new() -> Self {
        Self {
            input: Mutex::new(Box::new(BufReader::new(std::io::stdin()))),
            output: Mutex::new(Box::new(std::io::stderr())),
        }
    }
}

impl ToolAuthorizer for TerminalAuthorizer {
    fn authorize(&self, request: &AuthorizationRequest) -> Result<AuthorizationDecision, String> {
        let mut input = self
            .input
            .lock()
            .map_err(|_| "terminal approval input lock was poisoned".to_string())?;
        let mut output = self
            .output
            .lock()
            .map_err(|_| "terminal approval output lock was poisoned".to_string())?;
        writeln!(
            output,
            "\nAllow {}? {}\nArguments: {}",
            request.tool, request.reason, request.arguments
        )
        .map_err(|error| format!("failed to write terminal approval: {error}"))?;

        loop {
            write!(output, "Allow once? [y/N]: ")
                .and_then(|()| output.flush())
                .map_err(|error| format!("failed to write terminal approval: {error}"))?;
            let mut response = String::new();
            let read = input
                .read_line(&mut response)
                .map_err(|error| format!("failed to read terminal approval: {error}"))?;
            if read == 0 {
                writeln!(output, "Denied (input closed).").map_err(|error| {
                    format!("failed to write terminal approval result: {error}")
                })?;
                return Ok(AuthorizationDecision::Deny);
            }
            match response.trim().to_ascii_lowercase().as_str() {
                "y" | "yes" => return Ok(AuthorizationDecision::Allow),
                "" | "n" | "no" => return Ok(AuthorizationDecision::Deny),
                _ => writeln!(output, "Please answer y or n.")
                    .map_err(|error| format!("failed to write terminal approval: {error}"))?,
            }
        }
    }
}
