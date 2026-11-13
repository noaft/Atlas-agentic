//! Shared resource policy for commands started by an Atlas runtime or desktop app.
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
pub struct ProcessLimits {
    pub per_chat: usize,
    pub total: usize,
}

impl Default for ProcessLimits {
    fn default() -> Self {
        Self {
            per_chat: 8,
            total: 32,
        }
    }
}

impl ProcessLimits {
    pub const MAX_PER_CHAT: usize = 32;
    pub const MAX_TOTAL: usize = 128;

    pub fn validate(self) -> Result<(), String> {
        if !(1..=Self::MAX_PER_CHAT).contains(&self.per_chat) {
            return Err(format!(
                "Concurrent commands per chat must be between 1 and {}",
                Self::MAX_PER_CHAT
            ));
        }
        if !(1..=Self::MAX_TOTAL).contains(&self.total) {
            return Err(format!(
                "Concurrent commands across the app must be between 1 and {}",
                Self::MAX_TOTAL
            ));
        }
        Ok(())
    }
}
