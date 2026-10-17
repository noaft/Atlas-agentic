pub mod agent_output;
pub mod agents;
pub mod app;
pub mod conversations;
pub mod media;
pub mod models;
pub mod permissions;
pub mod runtime;
pub mod settings;
pub mod tasks;
pub mod tools;
pub mod workspace;

pub use runtime::{Runtime, RuntimeAskCallbacks};

use conversations::{Conversation, ConversationId, ConversationMetadata, MessageRole};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AppResult {
    Help,
    Ask {
        prompt: String,
        conversation_id: Option<ConversationId>,
    },
    ConversationList,
    ConversationCompact {
        id: ConversationId,
    },
    ConversationShow {
        id: ConversationId,
    },
    ConversationRename {
        id: ConversationId,
        title: String,
    },
    ConversationDelete {
        id: ConversationId,
    },
}

pub fn handle_command<I, S>(args: I) -> Result<AppResult, String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let mut args = args.into_iter().map(|arg| arg.as_ref().to_string());

    match args.next().as_deref() {
        None | Some("help") | Some("--help") | Some("-h") => Ok(AppResult::Help),
        Some("ask") => parse_ask(args),
        Some("conversation") => parse_conversation(args),
        Some(command) => Err(format!("unknown command: {command}")),
    }
}

fn parse_ask(mut args: impl Iterator<Item = String>) -> Result<AppResult, String> {
    let mut conversation_id = None;
    let mut prompt_parts = Vec::new();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--stream" => {}
            "--conversation" => {
                if conversation_id.is_some() {
                    return Err("--conversation can only be supplied once".to_string());
                }
                let id = args
                    .next()
                    .filter(|id| !id.starts_with('-'))
                    .ok_or_else(|| "--conversation needs an ID".to_string())?;
                conversation_id = Some(ConversationId::parse(&id)?);
            }
            option if option.starts_with('-') => {
                return Err(format!("unknown ask option: {option}"));
            }
            _ => {
                prompt_parts.push(arg);
                prompt_parts.extend(args);
                break;
            }
        }
    }
    let prompt = prompt_parts.join(" ");
    if prompt.trim().is_empty() {
        return Err("ask needs a prompt".to_string());
    }
    Ok(AppResult::Ask {
        prompt,
        conversation_id,
    })
}

fn parse_conversation(mut args: impl Iterator<Item = String>) -> Result<AppResult, String> {
    let command = args
        .next()
        .ok_or_else(|| "conversation needs a command: list, show, rename, or delete".to_string())?;
    let result = match command.as_str() {
        "list" => AppResult::ConversationList,
        "show" | "rename" | "delete" | "compact" => {
            let value = args
                .next()
                .ok_or_else(|| format!("conversation {command} needs an ID"))?;
            let id = ConversationId::parse(&value)?;
            match command.as_str() {
                "compact" => AppResult::ConversationCompact { id },
                "show" => AppResult::ConversationShow { id },
                "delete" => AppResult::ConversationDelete { id },
                _ => {
                    let title = args
                        .next()
                        .filter(|title| !title.trim().is_empty())
                        .ok_or_else(|| "conversation rename needs a title".to_string())?;
                    AppResult::ConversationRename { id, title }
                }
            }
        }
        _ => return Err(format!("unknown conversation command: {command}")),
    };
    if let Some(extra) = args.next() {
        return Err(format!(
            "unexpected argument for conversation {command}: {extra}"
        ));
    }
    Ok(result)
}

pub fn render_help() -> String {
    [
        "Atlas Agentic",
        "",
        "Usage:",
        "  atlas ask [--stream] [--conversation <id>] <prompt>",
        "  atlas conversation list",
        "  atlas conversation compact <id>",
        "  atlas conversation show <id>",
        "  atlas conversation rename <id> <title>",
        "  atlas conversation delete <id>",
        "  atlas help",
        "",
        "Responses stream by default. --stream remains available as an alias.",
        "Options go before the prompt. Quote prompts and titles containing spaces.",
        "Each ask starts a new conversation unless --conversation selects an existing ID.",
        "List/show/rename/delete use local data without a model key; compact requires model configuration.",
    ]
    .join("\n")
}

pub fn render_conversation_metadata(metadata: &ConversationMetadata) -> String {
    let title = metadata
        .title
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    format!("Conversation: {} | {title}", metadata.id)
}

pub fn render_conversation_summary(metadata: &ConversationMetadata) -> String {
    format!(
        "{}\nCreated: {}\nUpdated: {}\nMessages: {}",
        render_conversation_metadata(metadata),
        metadata.created_at.to_rfc3339(),
        metadata.updated_at.to_rfc3339(),
        metadata.message_count
    )
}

pub fn render_conversation(conversation: &Conversation) -> String {
    let mut output = render_conversation_summary(&conversation.metadata);
    for message in &conversation.messages {
        let role = match message.role {
            MessageRole::User => "user",
            MessageRole::Assistant => "assistant",
        };
        output.push_str(&format!(
            "\n\n{role} [{}]\n{}",
            message.created_at.to_rfc3339(),
            message.content
        ));
    }
    output
}
