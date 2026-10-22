use super::{
    native::{self, StreamState, WireResponse},
    ModelRequest,
};
use crate::settings::EffectiveSettings;
use serde_json::{json, Value};
pub(super) fn payload(config: &EffectiveSettings, request: &ModelRequest) -> Value {
    let mut contents: Vec<Value> = Vec::new();
    for (role, text) in native::messages(request) {
        let role = if role == "assistant" { "model" } else { "user" };
        if let Some(last) = contents.last_mut().filter(|m| m["role"] == role) {
            last["parts"]
                .as_array_mut()
                .expect("parts")
                .push(json!({"text":text}));
        } else {
            contents.push(json!({"role":role,"parts":[{"text":text}]}));
        }
    }
    if let Some(last) = contents.last_mut() {
        for image in request.images {
            last["parts"].as_array_mut().expect("parts").push(json!({"inlineData":{"mimeType":image.mime_type,"data":native::image_base64(image)}}));
        }
    }
    let mut body = json!({"systemInstruction":{"parts":[{"text":super::prompt::render_model_instructions(request)}]},"contents":contents,"generationConfig":{"maxOutputTokens":native::bound(config,request)}});
    if config.capabilities.json_mode == Some(true) && !request.tools.is_empty() {
        body["generationConfig"]["responseMimeType"] = json!("application/json");
    }
    body
}
fn candidate(raw: &Value) -> Result<&Value, String> {
    if raw.get("error").is_some_and(|error| !error.is_null())
        || raw["promptFeedback"].get("blockReason").is_some()
    {
        return Err("Gemini response failed or was blocked".into());
    }
    raw["candidates"]
        .as_array()
        .and_then(|v| v.first())
        .ok_or_else(|| "Gemini response returned no candidates".into())
}
fn text(choice: &Value) -> String {
    choice["content"]["parts"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|p| p["thought"] != true)
        .filter_map(|p| p["text"].as_str())
        .collect()
}
fn finish(reason: &str) -> String {
    match reason {
        "STOP" => "stop",
        "MAX_TOKENS" => "length",
        _ => "error",
    }
    .into()
}
pub(super) fn response(config: &EffectiveSettings, raw: &Value) -> Result<WireResponse, String> {
    let choice = candidate(raw)?;
    Ok(WireResponse {
        text: text(choice),
        usage: native::usage(config, &raw["usageMetadata"]),
        finish: finish(choice["finishReason"].as_str().unwrap_or("")),
    })
}
pub(super) fn event(raw: &Value, state: &mut StreamState) -> Result<(), String> {
    if raw["candidates"].as_array().is_some_and(|v| !v.is_empty()) {
        let choice = candidate(raw)?;
        state.text.push_str(&text(choice));
        if let Some(reason) = choice["finishReason"].as_str() {
            state.finish = finish(reason);
            state.terminal = true;
        }
    } else if raw.get("promptFeedback").is_some() || raw.get("usageMetadata").is_none() {
        return Err("Gemini stream returned no candidates".into());
    }
    if raw.get("usageMetadata").is_some() {
        state.raw_usage = raw["usageMetadata"].clone();
    }
    Ok(())
}
