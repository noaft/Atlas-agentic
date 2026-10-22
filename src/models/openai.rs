use super::{
    native::{self, StreamState, WireResponse},
    ModelRequest,
};
use crate::settings::EffectiveSettings;
use serde_json::{json, Value};
pub(super) fn payload(config: &EffectiveSettings, request: &ModelRequest, stream: bool) -> Value {
    let mut input: Vec<Value> = native::messages(request)
        .into_iter()
        .map(|(role, text)| json!({"role":role,"content":text}))
        .collect();
    if !request.images.is_empty() {
        let last = input.last_mut().expect("current user");
        let mut content = vec![json!({"type":"input_text","text":request.user_prompt})];
        for image in request.images {
            content.push(json!({"type":"input_image","image_url":native::data_url(image)}));
        }
        last["content"] = json!(content);
    }
    let mut body = json!({"model":config.model,"instructions":super::prompt::render_model_instructions(request),"input":input,"stream":stream,"store":false,"max_output_tokens":native::bound(config,request)});
    if config.capabilities.json_mode == Some(true) && !request.tools.is_empty() {
        body["text"] = json!({"format":{"type":"json_object"}});
    }
    body
}
pub(super) fn response(config: &EffectiveSettings, raw: &Value) -> Result<WireResponse, String> {
    if raw.get("error").is_some_and(|error| !error.is_null()) {
        return Err("OpenAI response returned an error".into());
    }
    let mut text = String::new();
    for item in raw["output"]
        .as_array()
        .ok_or("OpenAI response missing output")?
    {
        if item["type"] == "message" {
            for part in item["content"].as_array().into_iter().flatten() {
                if part["type"] == "output_text" {
                    text.push_str(part["text"].as_str().unwrap_or(""));
                }
            }
        }
    }
    Ok(WireResponse {
        text,
        usage: native::usage(config, &raw["usage"]),
        finish: finish(raw),
    })
}
fn finish(raw: &Value) -> String {
    match raw["status"].as_str() {
        Some("completed") => "stop",
        Some("incomplete") if raw["incomplete_details"]["reason"] == "max_output_tokens" => {
            "length"
        }
        _ => "error",
    }
    .into()
}
pub(super) fn event(raw: &Value, state: &mut StreamState) -> Result<(), String> {
    match raw["type"].as_str().unwrap_or("") {
        "response.output_text.delta" => state
            .text
            .push_str(raw["delta"].as_str().ok_or("OpenAI invalid text delta")?),
        "response.completed" => {
            state.raw_usage = raw["response"]["usage"].clone();
            state.finish = finish(&raw["response"]);
            state.terminal = true;
        }
        "response.failed" | "response.incomplete" | "response.refusal.delta" => {
            return Err("OpenAI stream was refused, failed or truncated".into())
        }
        _ => {}
    }
    Ok(())
}
