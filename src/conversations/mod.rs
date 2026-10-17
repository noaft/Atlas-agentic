mod activity;
mod service;
mod store;
mod title;
mod types;

pub use activity::{
    render_activity_context, sanitize_arguments, sanitize_result, ActivityCollector, ActivityKind,
    ActivityOutcome, ActivityRecord, ActivityTurn, ACTIVITY_CONTEXT_LIMIT, ACTIVITY_VERSION,
    ARGUMENT_LIMIT, RESULT_DETAIL_LIMIT, TURN_DETAIL_LIMIT,
};
pub use service::{AskCallbacks, ConversationAskResult, ConversationService};
pub use store::{validate_workspace, ConversationStore};
pub use title::sanitize_title;
pub use types::{
    history_limit_from_env, select_recent_history, Conversation, ConversationId,
    ConversationMessage, ConversationMetadata, MessageRole, DEFAULT_HISTORY_MESSAGES,
};

mod context;
mod memory;
pub use context::{choose_block_count, retained_start, ContextPolicy};
pub use memory::{
    CompactionProgress, CompactionResult, CompactionTrigger, MemoryBlock, MemoryCheckpoint,
};
pub use store::memory::{memory_fingerprint, ConversationLease};

pub(crate) mod compaction;
