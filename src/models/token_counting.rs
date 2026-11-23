//! Provider count APIs projected from the same payload used for inference.
//! Only a digest and numeric count are cached; prompts and images are never retained.
use super::{native::model_path, TokenEstimate};
use crate::settings::{EffectiveSettings, ProviderId};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::sync::Mutex;

#[derive(Default)]
pub(super) struct CountCache(Mutex<Option<([u8; 32], TokenEstimate)>>);
impl CountCache {
    pub fn count(
        &self,
        config: &EffectiveSettings,
        payload: Value,
        interrupted: &dyn Fn() -> bool,
    ) -> Result<TokenEstimate, String> {
        if interrupted() {
            return Err("Token counting interrupted".into());
        }
        let (path, body, field) = match config.provider {
            ProviderId::Openai => {
                let mut body = json!({"model":payload["model"],"instructions":payload["instructions"],"input":payload["input"]});
                if let Some(text) = payload.get("text") {
                    body["text"] = text.clone();
                }
                ("responses/input_tokens".into(), body, "input_tokens")
            }
            ProviderId::Anthropic => (
                "messages/count_tokens".into(),
                json!({"model":payload["model"],"system":payload["system"],"messages":payload["messages"]}),
                "input_tokens",
            ),
            ProviderId::Gemini => {
                let mut request = payload;
                request["model"] = json!(format!(
                    "models/{}",
                    config
                        .model
                        .strip_prefix("models/")
                        .unwrap_or(&config.model)
                ));
                (
                    format!("{}:countTokens", model_path(&config.model)),
                    json!({"generateContentRequest":request}),
                    "totalTokens",
                )
            }
            _ => return Err("Provider token counting is unavailable".into()),
        };
        let key: [u8; 32] = Sha256::digest(
            serde_json::to_vec(&body).map_err(|_| "Could not encode token count request")?,
        )
        .into();
        if let Some((_, count)) = self
            .0
            .lock()
            .map_err(|_| "Token count cache unavailable")?
            .as_ref()
            .filter(|(cached, _)| *cached == key)
        {
            return Ok(*count);
        }
        let url = format!("{}/{path}", config.base_url.trim_end_matches('/'));
        let raw = super::streaming::read_json_response(
            |client| {
                let builder = client.post(url);
                let builder = match config.provider {
                    ProviderId::Anthropic => builder
                        .header("x-api-key", config.api_key.as_deref().unwrap_or_default())
                        .header(
                            "anthropic-version",
                            config.provider.descriptor().api_version.unwrap_or_default(),
                        ),
                    ProviderId::Gemini => builder.header(
                        "x-goog-api-key",
                        config.api_key.as_deref().unwrap_or_default(),
                    ),
                    _ => builder.bearer_auth(config.api_key.as_deref().unwrap_or_default()),
                };
                builder.json(&body)
            },
            config.provider.as_str(),
            interrupted,
        )?;
        let tokens = raw
            .get(field)
            .and_then(Value::as_u64)
            .filter(|_| raw.get("error").is_none_or(Value::is_null))
            .ok_or_else(|| {
                format!(
                    "{} token count response was invalid",
                    config.provider.as_str()
                )
            })?;
        if interrupted() {
            return Err("Token counting interrupted".into());
        }
        let count = TokenEstimate {
            tokens,
            estimated: config.provider == ProviderId::Anthropic,
        };
        *self.0.lock().map_err(|_| "Token count cache unavailable")? = Some((key, count));
        Ok(count)
    }
}
