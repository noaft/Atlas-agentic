use super::{ConversationId, ConversationMessage};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CompactionTrigger {
    Manual,
    Automatic,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryBlock {
    pub start_index: usize,
    pub end_index_exclusive: usize,
    pub content: String,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryCheckpoint {
    pub version: u32,
    pub policy_version: u32,
    pub conversation_id: ConversationId,
    pub checkpoint_id: String,
    pub created_at: DateTime<Utc>,
    pub model: String,
    pub trigger: CompactionTrigger,
    pub occupancy_percent: f64,
    pub estimated: bool,
    pub requested_blocks: usize,
    pub compacted_message_count: usize,
    pub prefix_fingerprint: String,
    pub blocks: Vec<MemoryBlock>,
    pub summary_usage: Vec<crate::models::ModelUsage>,
}
impl MemoryCheckpoint {
    pub fn validate(
        &self,
        id: &ConversationId,
        messages: &[ConversationMessage],
    ) -> Result<(), String> {
        if self.version != 1
            || self.policy_version != 1
            || &self.conversation_id != id
            || self.blocks.is_empty()
            || self.blocks.len() > 3
            || self.requested_blocks == 0
            || self.requested_blocks > 3
            || self.blocks.len() > self.requested_blocks
            || self.compacted_message_count > messages.len()
            || self.compacted_message_count == 0
            || !self.occupancy_percent.is_finite()
            || self.occupancy_percent < 0.0
        {
            return Err("Invalid conversation memory checkpoint".into());
        }
        let mut end = 0;
        for block in &self.blocks {
            if block.start_index != end
                || block.end_index_exclusive <= end
                || block.content.trim().is_empty()
            {
                return Err("Conversation memory has empty, overlapping, or missing ranges".into());
            }
            end = block.end_index_exclusive;
        }
        if end != self.compacted_message_count
            || self.prefix_fingerprint != fingerprint(&messages[..end])?
        {
            return Err("Conversation memory does not match original history".into());
        }
        Ok(())
    }
}
pub fn fingerprint(messages: &[ConversationMessage]) -> Result<String, String> {
    use sha2::{Digest, Sha256};
    let bytes = serde_json::to_vec(messages).map_err(|e| e.to_string())?;
    Ok(format!("{:x}", Sha256::digest(bytes)))
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CompactionProgress {
    pub completed_blocks: usize,
    pub total_blocks: usize,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CompactionResult {
    pub memory: Option<MemoryCheckpoint>,
    pub message: String,
}
