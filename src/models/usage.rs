use super::ModelUsage;
use crate::settings::{ModelSource, ProviderId};
use serde_json::Value;
pub fn checked_usage(
    source: ModelSource,
    model: &str,
    input: u64,
    output: u64,
    total: Option<u64>,
) -> Option<ModelUsage> {
    let sum = input.checked_add(output)?;
    if total.is_some_and(|t| t != sum) {
        return None;
    }
    Some(ModelUsage {
        source: Some(source),
        model: model.into(),
        input_tokens: input,
        output_tokens: output,
        total_tokens: sum,
    })
}
fn optional(raw: &Value, field: &str) -> Option<u64> {
    raw.get(field).map_or(Some(0), Value::as_u64)
}
pub fn parse(source: &ModelSource, model: &str, raw: &Value) -> Option<ModelUsage> {
    let (input, output, total_field) = match source.provider {
        ProviderId::Deepseek | ProviderId::OpenaiCompatible => (
            raw.get("prompt_tokens")?.as_u64()?,
            raw.get("completion_tokens")?.as_u64()?,
            "total_tokens",
        ),
        ProviderId::Openai => (
            raw.get("input_tokens")?.as_u64()?,
            raw.get("output_tokens")?.as_u64()?,
            "total_tokens",
        ),
        ProviderId::Anthropic => (
            raw.get("input_tokens")?
                .as_u64()?
                .checked_add(optional(raw, "cache_read_input_tokens")?)?
                .checked_add(optional(raw, "cache_creation_input_tokens")?)?,
            raw.get("output_tokens")?.as_u64()?,
            "total_tokens",
        ),
        ProviderId::Gemini => (
            raw.get("promptTokenCount")?
                .as_u64()?
                .checked_add(optional(raw, "toolUsePromptTokenCount")?)?,
            raw.get("candidatesTokenCount")?
                .as_u64()?
                .checked_add(optional(raw, "thoughtsTokenCount")?)?,
            "totalTokenCount",
        ),
    };
    let total = raw.get(total_field).map(Value::as_u64).transpose_option()?;
    checked_usage(source.clone(), model, input, output, total)
}
trait TransposeOption {
    fn transpose_option(self) -> Option<Option<u64>>;
}
impl TransposeOption for Option<Option<u64>> {
    fn transpose_option(self) -> Option<Option<u64>> {
        match self {
            None => Some(None),
            Some(Some(v)) => Some(Some(v)),
            Some(None) => None,
        }
    }
}
