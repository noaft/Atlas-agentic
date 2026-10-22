use super::{
    native::{self, StreamState, WireResponse},
    ModelRequest,
};
use crate::settings::EffectiveSettings;
use serde_json::{json, Value};
pub(super) fn payload(config: &EffectiveSettings, request: &ModelRequest, stream: bool) -> Value {
    let mut messages =
        vec![json!({"role":"system","content":super::prompt::render_model_instructions(request)})];
    messages.extend(
        native::messages(request)
            .into_iter()
            .map(|(role, text)| json!({"role":role,"content":text})),
    );
    if !request.images.is_empty() {
        let last = messages.last_mut().expect("current user");
        let mut parts = vec![json!({"type":"text","text":request.user_prompt})];
        for image in request.images {
            parts.push(json!({"type":"image_url","image_url":{"url":native::data_url(image)}}));
        }
        last["content"] = json!(parts);
    }
    let mut body = json!({"model":config.model,"messages":messages,"stream":stream,"max_tokens":native::bound(config,request)});
    if config.capabilities.json_mode == Some(true) && !request.tools.is_empty() {
        body["response_format"] = json!({"type":"json_object"});
    }
    if stream && config.capabilities.stream_usage == Some(true) {
        body["stream_options"] = json!({"include_usage":true});
    }
    body
}
pub(super) fn response(config: &EffectiveSettings, raw: &Value) -> Result<WireResponse, String> {
    let choice = raw["choices"]
        .as_array()
        .and_then(|v| v.first())
        .ok_or("Compatible response returned no choices")?;
    Ok(WireResponse {
        text: choice["message"]["content"]
            .as_str()
            .ok_or("Compatible response missing text")?
            .into(),
        usage: native::usage(config, &raw["usage"]),
        finish: choice["finish_reason"].as_str().unwrap_or("error").into(),
    })
}
pub(super) fn event(raw: &Value, state: &mut StreamState) -> Result<(), String> {
    if let Some(choices) = raw["choices"].as_array() {
        if let Some(choice) = choices.first() {
            if let Some(text) = choice["delta"]["content"].as_str() {
                state.text.push_str(text);
            }
            if let Some(finish) = choice["finish_reason"].as_str() {
                state.finish = finish.into();
            }
        }
    } else {
        return Err("Compatible stream missing choices".into());
    }
    if raw.get("usage").is_some_and(|v| !v.is_null()) {
        state.raw_usage = raw["usage"].clone();
    }
    Ok(())
}
