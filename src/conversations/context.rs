use super::{CompactionTrigger, ConversationMessage, MessageRole};
use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct ContextPolicy {
    pub enabled: bool,
    pub auto_trigger_percent: u64,
    pub keep_recent_messages: usize,
    pub summary_max_tokens: u64,
    pub output_reserve_tokens: u64,
    pub max_reduction_passes: usize,
    pub max_summary_requests: usize,
}
impl Default for ContextPolicy {
    fn default() -> Self {
        Self {
            enabled: false,
            auto_trigger_percent: 90,
            keep_recent_messages: 10,
            summary_max_tokens: 2048,
            output_reserve_tokens: 4096,
            max_reduction_passes: 4,
            max_summary_requests: 24,
        }
    }
}
impl ContextPolicy {
    pub fn validate(&self) -> Result<(), String> {
        if !(1..100).contains(&self.auto_trigger_percent)
            || self.keep_recent_messages == 0
            || self.summary_max_tokens == 0
            || self.output_reserve_tokens == 0
            || self.max_reduction_passes == 0
            || self.max_summary_requests == 0
        {
            return Err("Memory policy needs a 1–99% threshold and positive budgets".into());
        }
        Ok(())
    }
}
pub fn choose_block_count(trigger: CompactionTrigger, percent: f64) -> usize {
    if trigger == CompactionTrigger::Automatic || percent >= 60.0 {
        3
    } else if percent >= 30.0 {
        2
    } else {
        1
    }
}
pub fn retained_start(messages: &[ConversationMessage], keep: usize) -> usize {
    let mut start = messages.len().saturating_sub(keep);
    while start > 0 && matches!(messages[start].role, MessageRole::Assistant) {
        start -= 1;
    }
    start
}
