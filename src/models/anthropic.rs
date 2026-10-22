use super::{
    native::{self, StreamState, WireResponse},
    ModelRequest,
};
use crate::settings::EffectiveSettings;
use serde_json::{json, Value};
pub(super) fn payload(config: &EffectiveSettings, request: &ModelRequest, stream: bool) -> Value {
    // Native Messages requires alternating content roles; combine adjacent history/memory roles.
    let mut messages: Vec<Value> = Vec::new();
    for (role, text) in native::messages(request) {
        if let Some(last) = messages.last_mut().filter(|m| m["role"] == role) {
            let prior = last["content"].as_str().unwrap_or("");
            last["content"] = json!(format!("{prior}\n\n{text}"));
        } else {
            messages.push(json!({"role":role,"content":text}));
        }
    }
    if !request.images.is_empty() {
        let last = messages.last_mut().expect("current user");
        let mut parts = vec![json!({"type":"text","text":last["content"]})];
        for image in request.images {
            parts.push(json!({"type":"image","source":{"type":"base64","media_type":image.mime_type,"data":native::image_base64(image)}}));
        }
        last["content"] = json!(parts);
    }
    json!({"model":config.model,"system":super::prompt::render_model_instructions(request),"messages":messages,"stream":stream,"max_tokens":native::bound(config,request)})
}
fn finish(reason: &str) -> String {
    match reason {
        "end_turn" | "stop_sequence" => "stop",
        "max_tokens" => "length",
        _ => "error",
    }
    .into()
}
pub(super) fn response(config: &EffectiveSettings, raw: &Value) -> Result<WireResponse, String> {
    let parts = raw["content"]
        .as_array()
        .ok_or("Anthropic response missing content")?;
    let text = parts
        .iter()
        .filter(|p| p["type"] == "text")
        .filter_map(|p| p["text"].as_str())
        .collect::<String>();
    Ok(WireResponse {
        text,
        usage: native::usage(config, &raw["usage"]),
        finish: finish(raw["stop_reason"].as_str().unwrap_or("")),
    })
}
pub(super) fn event(raw: &Value, state: &mut StreamState) -> Result<(), String> {
    match raw["type"].as_str().unwrap_or("") {
        "message_start" => {
            state.raw_usage = raw["message"]["usage"].clone();
        }
        "content_block_delta" if raw["delta"]["type"] == "text_delta" => state.text.push_str(
            raw["delta"]["text"]
                .as_str()
                .ok_or("Anthropic invalid text delta")?,
        ),
        "message_delta" => {
            if let Some(fields) = raw["usage"].as_object() {
                if !state.raw_usage.is_object() {
                    state.raw_usage = json!({});
                }
                for (k, v) in fields {
                    state.raw_usage[k] = v.clone();
                }
            }
            if let Some(reason) = raw["delta"]["stop_reason"].as_str() {
                state.finish = finish(reason);
            }
        }
        "message_stop" => state.terminal = true,
        _ => {}
    }
    Ok(())
}
