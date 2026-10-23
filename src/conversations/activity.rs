use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

pub const ACTIVITY_VERSION: u32 = 1;
pub const ARGUMENT_LIMIT: usize = 2_048;
pub const RESULT_DETAIL_LIMIT: usize = 8_192;
pub const TURN_DETAIL_LIMIT: usize = 32_768;
pub const ACTIVITY_CONTEXT_LIMIT: usize = 8_192;

const TRUNCATION_SUFFIX: &str = "… [truncated]";
const CONTEXT_HEADING: &str = "Recent Atlas activity (facts from earlier successful turns):\n";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ActivityKind {
    Progress,
    Status,
    ToolCall,
    ToolResult,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ActivityOutcome {
    Succeeded,
    Failed,
    Denied,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ActivityRecord {
    pub sequence: u32,
    pub kind: ActivityKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub arguments: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub outcome: Option<ActivityOutcome>,
    pub summary: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<String>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ActivityTurn {
    pub version: u32,
    pub turn_id: String,
    pub assistant_message_index: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage: Option<crate::models::ModelUsage>,
    pub records: Vec<ActivityRecord>,
}

#[derive(Debug, Default)]
pub struct ActivityCollector {
    records: Vec<ActivityRecord>,
    detail_characters: usize,
}

impl ActivityCollector {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn status(&mut self, summary: impl Into<String>) {
        self.push(ActivityRecord {
            sequence: 0,
            kind: ActivityKind::Status,
            tool: None,
            arguments: None,
            outcome: None,
            summary: summary.into(),
            details: None,
            created_at: Utc::now(),
        });
    }

    pub fn progress(&mut self, message: &str) {
        self.push(ActivityRecord {
            sequence: 0,
            kind: ActivityKind::Progress,
            tool: None,
            arguments: None,
            outcome: None,
            summary: format!("Atlas progress (not a completed action): {message}"),
            details: None,
            created_at: Utc::now(),
        });
    }

    pub fn tool_requested(&mut self, tool: &str, args: &Value) {
        self.tool_requested_sanitized(tool, sanitize_arguments(args));
    }

    pub fn tool_requested_sanitized(&mut self, tool: &str, arguments: String) {
        self.push(ActivityRecord {
            sequence: 0,
            kind: ActivityKind::ToolCall,
            tool: Some(tool.to_string()),
            arguments: Some(arguments),
            outcome: None,
            summary: format!("Requested {tool}"),
            details: None,
            created_at: Utc::now(),
        });
    }

    pub fn tool_denied(&mut self, tool: &str) {
        self.push(ActivityRecord {
            sequence: 0,
            kind: ActivityKind::ToolResult,
            tool: Some(tool.to_string()),
            arguments: None,
            outcome: Some(ActivityOutcome::Denied),
            summary: format!("{tool} was denied"),
            details: None,
            created_at: Utc::now(),
        });
    }

    pub fn tool_finished(
        &mut self,
        tool: &str,
        success: bool,
        summary: impl Into<String>,
        details: &str,
    ) {
        let details = if tool == "capture_screen" {
            None
        } else {
            self.bounded_details(details)
        };
        self.push(ActivityRecord {
            sequence: 0,
            kind: ActivityKind::ToolResult,
            tool: Some(tool.to_string()),
            arguments: None,
            outcome: Some(if success {
                ActivityOutcome::Succeeded
            } else {
                ActivityOutcome::Failed
            }),
            summary: summary.into(),
            details,
            created_at: Utc::now(),
        });
    }

    pub fn records(&self) -> &[ActivityRecord] {
        &self.records
    }

    pub fn into_turn(
        self,
        turn_id: impl Into<String>,
        assistant_message_index: usize,
    ) -> ActivityTurn {
        ActivityTurn {
            version: ACTIVITY_VERSION,
            turn_id: turn_id.into(),
            assistant_message_index,
            duration_ms: None,
            usage: None,
            records: self.records,
        }
    }

    pub fn into_unassociated_turn(self) -> ActivityTurn {
        self.into_turn(Uuid::new_v4().to_string(), 0)
    }

    fn bounded_details(&mut self, details: &str) -> Option<String> {
        let remaining = TURN_DETAIL_LIMIT.saturating_sub(self.detail_characters);
        if remaining == 0 || details.is_empty() {
            return None;
        }
        let limit = remaining.min(RESULT_DETAIL_LIMIT);
        let bounded = truncate_chars(details, limit);
        self.detail_characters += bounded.chars().count();
        Some(bounded)
    }

    fn push(&mut self, mut record: ActivityRecord) {
        record.sequence = self.records.len() as u32;
        self.records.push(record);
    }
}

pub fn sanitize_arguments(args: &Value) -> String {
    let sanitized = redact_value(args, None);
    let rendered = serde_json::to_string(&sanitized).unwrap_or_else(|_| "{}".to_string());
    truncate_chars(&rendered, ARGUMENT_LIMIT)
}

pub fn sanitize_result(result: &str) -> String {
    truncate_chars(result, RESULT_DETAIL_LIMIT)
}

pub fn render_activity_context(turns: &[ActivityTurn]) -> String {
    let mut selected = Vec::new();
    let mut used = CONTEXT_HEADING.chars().count();
    for record in turns
        .iter()
        .rev()
        .flat_map(|turn| turn.records.iter().rev())
    {
        let line = format!("- {}\n", record.summary);
        let length = line.chars().count();
        if used + length <= ACTIVITY_CONTEXT_LIMIT {
            selected.push(line);
            used += length;
        } else {
            break;
        }
    }
    if selected.is_empty() {
        return String::new();
    }
    selected.reverse();
    format!("{CONTEXT_HEADING}{}", selected.concat())
        .trim_end()
        .to_string()
}

fn truncate_chars(value: &str, limit: usize) -> String {
    if value.chars().count() <= limit {
        return value.to_string();
    }
    let suffix_length = TRUNCATION_SUFFIX.chars().count();
    let mut bounded = value
        .chars()
        .take(limit.saturating_sub(suffix_length))
        .collect::<String>();
    bounded.push_str(TRUNCATION_SUFFIX);
    bounded
}

fn redact_value(value: &Value, key: Option<&str>) -> Value {
    if key.is_some_and(secret_key) {
        return Value::String("[redacted]".to_string());
    }
    match value {
        Value::Object(values) => Value::Object(
            values
                .iter()
                .map(|(key, value)| (key.clone(), redact_value(value, Some(key))))
                .collect(),
        ),
        Value::Array(values) => Value::Array(
            values
                .iter()
                .map(|value| redact_value(value, None))
                .collect(),
        ),
        _ => value.clone(),
    }
}

fn secret_key(key: &str) -> bool {
    let normalized = key.to_ascii_lowercase();
    ["key", "token", "password", "secret", "authorization"]
        .iter()
        .any(|part| normalized.contains(part))
}
