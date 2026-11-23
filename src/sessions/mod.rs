//! Durable session traces are independent from committed conversation history.
pub(crate) mod context;
mod events;
mod io;
mod store;
pub(crate) mod trace;
mod writer;
use crate::conversations::ConversationId;
use chrono::{DateTime, FixedOffset, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
pub use store::SessionStore;
pub use writer::SessionJournal;
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ContextCursor {
    pub history_message_start: usize,
    pub checkpoint_id: Option<String>,
    pub prepared_event: Option<u64>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SessionMetadata {
    pub version: u32,
    pub id: ConversationId,
    pub first_sent_at: DateTime<FixedOffset>,
    pub last_sequence: u64,
    pub active_turn: Option<String>,
    #[serde(default)]
    pub last_finished_turn: Option<String>,
    pub context_cursor: ContextCursor,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SessionEvent {
    pub version: u32,
    pub session_id: ConversationId,
    pub turn_id: String,
    pub sequence: u64,
    pub timestamp: DateTime<Utc>,
    pub kind: String,
    pub data: Value,
    pub truncated: bool,
}
