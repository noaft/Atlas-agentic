use std::{env, fmt, path::PathBuf};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::permissions::PermissionMode;

use super::ActivityTurn;

pub const DEFAULT_HISTORY_MESSAGES: usize = 40;

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ConversationId(Uuid);

impl Default for ConversationId {
    fn default() -> Self {
        Self::new()
    }
}

impl ConversationId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }

    pub fn parse(value: &str) -> Result<Self, String> {
        Uuid::parse_str(value)
            .map(Self)
            .map_err(|error| format!("invalid conversation ID: {error}"))
    }
}

impl fmt::Display for ConversationId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MessageRole {
    User,
    Assistant,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConversationMessage {
    pub role: MessageRole,
    pub content: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConversationMetadata {
    pub id: ConversationId,
    pub title: String,
    pub title_attempted: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub message_count: usize,
    #[serde(rename = "workspacePath", default)]
    pub workspace_path: Option<PathBuf>,
    #[serde(rename = "permissionMode", default)]
    pub permission_mode: Option<PermissionMode>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Conversation {
    pub memory: Option<super::MemoryCheckpoint>,
    pub metadata: ConversationMetadata,
    pub messages: Vec<ConversationMessage>,
    pub activities: Vec<ActivityTurn>,
}

pub fn history_limit_from_env() -> Result<usize, String> {
    let Some(value) = env::var_os("ATLAS_HISTORY_MESSAGES") else {
        return Ok(DEFAULT_HISTORY_MESSAGES);
    };

    let value = value
        .into_string()
        .map_err(|_| "ATLAS_HISTORY_MESSAGES must be a positive integer".to_string())?;
    let limit = value
        .parse::<usize>()
        .map_err(|_| "ATLAS_HISTORY_MESSAGES must be a positive integer".to_string())?;

    if limit == 0 {
        return Err("ATLAS_HISTORY_MESSAGES must be a positive integer".to_string());
    }

    Ok(limit)
}

pub fn select_recent_history(
    messages: &[ConversationMessage],
    limit: usize,
) -> Vec<ConversationMessage> {
    let mut start = messages.len().saturating_sub(limit);
    if matches!(
        messages.get(start).map(|message| &message.role),
        Some(MessageRole::User)
    ) {
        while start > 0 && matches!(messages[start - 1].role, MessageRole::User) {
            start -= 1;
        }
    }
    let mut recent = messages[start..].to_vec();

    if start > 0
        && matches!(
            recent.first().map(|message| &message.role),
            Some(MessageRole::Assistant)
        )
    {
        recent.remove(0);
    }

    recent
}

/// Annotate model-facing history copies; durable history and its fingerprints stay unchanged.
pub fn annotate_interrupted_history(
    messages: &mut [ConversationMessage],
    first_index: usize,
    activities: &[ActivityTurn],
) {
    const LABEL: &str = "[Interrupted assistant response: this partial text is incomplete and does not establish task completion.]\n";
    for activity in activities.iter().filter(|activity| activity.interrupted) {
        let Some(index) = activity.assistant_message_index.checked_sub(first_index) else {
            continue;
        };
        if let Some(message) = messages.get_mut(index) {
            if matches!(message.role, MessageRole::Assistant) && !message.content.starts_with(LABEL)
            {
                message.content.insert_str(0, LABEL);
            }
        }
    }
}
