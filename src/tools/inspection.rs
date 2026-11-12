use serde::{Deserialize, Serialize};

pub(crate) const MAX_OUTPUT_BYTES: usize = 64 * 1024;

#[derive(Clone, Copy, Default, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum OutputFormat {
    #[default]
    Text,
    Json,
}

pub(crate) fn page_message(
    data: impl Serialize,
    truncated: bool,
    next_cursor: Option<serde_json::Value>,
) -> Result<String, String> {
    #[derive(Serialize)]
    struct Page<T> {
        data: T,
        truncated: bool,
        next_cursor: Option<serde_json::Value>,
    }
    serde_json::to_string(&Page {
        data,
        truncated,
        next_cursor,
    })
    .map_err(|error| format!("failed to encode tool page: {error}"))
}
