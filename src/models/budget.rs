use serde::{Deserialize, Serialize};
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelLimits {
    pub context_window: u64,
    pub max_output_tokens: u64,
}
impl ModelLimits {
    pub fn validate(&self) -> Result<(), String> {
        if self.context_window == 0
            || self.max_output_tokens == 0
            || self.max_output_tokens >= self.context_window
        {
            return Err(
                "Model limits must be positive and output must be smaller than context".into(),
            );
        }
        Ok(())
    }
}
#[derive(Debug, Clone, Copy)]
pub struct TokenEstimate {
    pub tokens: u64,
    pub estimated: bool,
}
