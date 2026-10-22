use super::{
    native::{auth, checked_response, client, validate_profile},
    ModelInfo,
};
use crate::settings::{EffectiveSettings, ProviderCapabilities, ProviderId};
use serde_json::Value;
use std::collections::BTreeSet;

/// List a provider's catalog without constructing a generation client or requiring a model.
pub fn discover_models(settings: &EffectiveSettings) -> Result<Vec<ModelInfo>, String> {
    crate::diagnostics::provider_settings(settings);
    validate_profile(settings)?;
    let client = client();
    let endpoint = format!("{}/models", settings.base_url.trim_end_matches('/'));
    let mut cursor = None::<String>;
    let mut cursors = BTreeSet::new();
    let mut ids = BTreeSet::new();
    let mut models = Vec::new();
    for _ in 0..100 {
        let mut request = auth(client.get(&endpoint), settings);
        if let Some(cursor) = &cursor {
            request = request.query(&[(
                if settings.provider == ProviderId::Gemini {
                    "pageToken"
                } else {
                    "after_id"
                },
                cursor,
            )]);
        }
        let response = checked_response(request.send(), settings, "model discovery")?;
        let raw: Value = response
            .json()
            .map_err(|_| format!("{} model list was invalid", settings.provider.as_str()))?;
        let items = raw[if settings.provider == ProviderId::Gemini {
            "models"
        } else {
            "data"
        }]
        .as_array()
        .ok_or("Provider model list missing models")?;
        for model in items {
            if settings.provider == ProviderId::Gemini
                && model["supportedGenerationMethods"]
                    .as_array()
                    .is_some_and(|methods| !methods.iter().any(|m| m == "generateContent"))
            {
                continue;
            }
            let id = model[if settings.provider == ProviderId::Gemini {
                "name"
            } else {
                "id"
            }]
            .as_str()
            .unwrap_or("")
            .trim();
            if id.is_empty() || !ids.insert(id.to_string()) {
                continue;
            }
            let positive = |key: &str| model[key].as_u64().filter(|v| *v > 0);
            let (input, context, output, name) = match settings.provider {
                ProviderId::Gemini => (
                    positive("inputTokenLimit"),
                    None,
                    positive("outputTokenLimit"),
                    model["displayName"].as_str(),
                ),
                ProviderId::Anthropic => (
                    positive("max_input_tokens"),
                    None,
                    positive("max_tokens"),
                    model["display_name"].as_str(),
                ),
                _ => (
                    positive("input_token_limit"),
                    positive("context_window"),
                    positive("max_output_tokens"),
                    model["name"].as_str(),
                ),
            };
            let capabilities = ProviderCapabilities {
                image_input: model["capabilities"]["image_input"]["supported"].as_bool(),
                temperature: model["capabilities"]["temperature"]["supported"].as_bool(),
                top_p: model["capabilities"]["top_p"]["supported"].as_bool(),
                ..Default::default()
            };
            models.push(ModelInfo {
                source: settings.source.clone(),
                input_token_limit: input,
                capabilities,
                id: id.into(),
                name: name.filter(|v| !v.trim().is_empty()).unwrap_or(id).into(),
                context_window: context,
                max_output_tokens: output,
            });
        }
        let next = if settings.provider == ProviderId::Gemini {
            raw["nextPageToken"].as_str()
        } else if raw["has_more"] == true {
            Some(
                raw["last_id"]
                    .as_str()
                    .ok_or("Provider model list missing pagination cursor")?,
            )
        } else {
            None
        };
        let Some(next) = next.filter(|s| !s.is_empty()) else {
            return Ok(models);
        };
        if !cursors.insert(next.to_string()) {
            return Err("Provider model discovery repeated a pagination cursor".into());
        }
        cursor = Some(next.to_string());
    }
    Err("Provider model discovery exceeded 100 pages".into())
}
