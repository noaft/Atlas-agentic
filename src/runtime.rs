pub mod context;
use serde::Serialize;

use crate::{
    agent_output::AgentOutput,
    agents::AgentRegistry,
    conversations::{sanitize_arguments, sanitize_result, ConversationMessage},
    media::ImageData,
    permissions::{
        requires_confirmation, AuthorizationDecision, AuthorizationRequest, MissingAuthorizer,
        PermissionMode, RuntimeStatus, ToolAuthorizer, ToolEvent,
    },
    tools::{ToolAttachment, ToolRegistry},
};

#[derive(Serialize)]
struct ToolExecutionRecord {
    tool: String,
    args: serde_json::Value,
    success: bool,
    output: String,
}

#[derive(Serialize)]
struct ToolContinuation<'a> {
    #[serde(rename = "type")]
    message_type: &'static str,
    original_user_request: &'a str,
    tool_results: &'a [ToolExecutionRecord],
    instruction: &'static str,
}

#[derive(Serialize)]
struct ProtocolRepair<'a> {
    #[serde(rename = "type")]
    message_type: &'static str,
    pending_request: &'a str,
    invalid_response: &'a str,
    validation_error: &'a str,
    attempt: usize,
    instruction: &'static str,
}

pub struct Runtime {
    agents: AgentRegistry,
    tools: ToolRegistry,
}

pub struct RuntimeAskCallbacks<'a> {
    pub authorizer: &'a dyn ToolAuthorizer,
    pub on_delta: &'a mut dyn FnMut(&str),
    pub on_tool_event: &'a mut dyn FnMut(&ToolEvent),
}

impl Runtime {
    pub fn new(agents: AgentRegistry) -> Self {
        Self {
            agents,
            tools: ToolRegistry::empty(),
        }
    }

    pub fn with_tools(mut self, tools: ToolRegistry) -> Self {
        self.tools = tools;
        self
    }

    pub fn default_agent(&self) -> Result<&dyn crate::agents::Agent, String> {
        self.agents
            .default_agent()
            .ok_or_else(|| "no agent is registered".into())
    }
    pub fn tool_definitions(&self) -> Vec<crate::tools::ToolDefinition> {
        self.tools.definitions()
    }
    pub fn generate_conversation_title(&self, first_input: &str) -> Result<String, String> {
        self.agents
            .default_agent()
            .ok_or_else(|| "no agent is registered".to_string())?
            .generate_conversation_title(first_input)
    }

    pub fn ask_stream(
        &self,
        prompt: &str,
        on_delta: &mut dyn FnMut(&str),
    ) -> Result<String, String> {
        self.ask_stream_with_history(prompt, &[], on_delta)
    }

    pub fn ask_stream_with_history(
        &self,
        prompt: &str,
        history: &[ConversationMessage],
        on_delta: &mut dyn FnMut(&str),
    ) -> Result<String, String> {
        self.ask_stream_with_history_and_authorization(
            prompt,
            history,
            PermissionMode::Ask,
            &MissingAuthorizer,
            on_delta,
            &mut |_| {},
        )
    }

    pub fn ask_stream_with_history_and_authorization(
        &self,
        prompt: &str,
        history: &[ConversationMessage],
        permission_mode: PermissionMode,
        authorizer: &dyn ToolAuthorizer,
        on_delta: &mut dyn FnMut(&str),
        on_tool_event: &mut dyn FnMut(&ToolEvent),
    ) -> Result<String, String> {
        self.ask_stream_with_context_and_authorization(
            prompt,
            history,
            None,
            permission_mode,
            RuntimeAskCallbacks {
                authorizer,
                on_delta,
                on_tool_event,
            },
        )
    }

    pub fn ask_stream_with_context_and_authorization(
        &self,
        prompt: &str,
        history: &[ConversationMessage],
        activity_context: Option<&str>,
        permission_mode: PermissionMode,
        callbacks: RuntimeAskCallbacks<'_>,
    ) -> Result<String, String> {
        let mut context = context::StaticContext {
            history,
            activity_context,
        };
        self.ask_stream_with_managed_context(prompt, &mut context, permission_mode, callbacks)
    }

    pub fn ask_stream_with_managed_context(
        &self,
        prompt: &str,
        context: &mut dyn context::RuntimeContext,
        permission_mode: PermissionMode,
        callbacks: RuntimeAskCallbacks<'_>,
    ) -> Result<String, String> {
        let RuntimeAskCallbacks {
            authorizer,
            on_delta,
            on_tool_event,
        } = callbacks;
        let agent = self
            .agents
            .default_agent()
            .ok_or_else(|| "no agent is registered".to_string())?;
        let tool_definitions = self.tools.definitions();
        let max_tool_calls = agent.max_tool_calls();
        let max_protocol_repairs = agent.max_protocol_repairs();

        let mut pending_request = prompt.to_string();
        let mut input = pending_request.clone();
        let mut tool_results = Vec::new();
        let mut latest_images: Vec<ImageData> = Vec::new();
        let mut protocol_repairs = 0_usize;
        let allow_plain_final = tool_definitions.is_empty();
        let mut first_model_request = true;
        let mut latest_usage = None;

        loop {
            on_tool_event(&ToolEvent::Status {
                status: if first_model_request {
                    RuntimeStatus::Thinking
                } else {
                    RuntimeStatus::PreparingAnswer
                },
            });
            first_model_request = false;
            let mut final_stream = FinalContentStream::new(allow_plain_final);
            let prepared = context.prepare(
                agent,
                context::ContextPreparationRequest {
                    input: &input,
                    tools: &tool_definitions,
                    images: &latest_images,
                    latest_usage: latest_usage.as_ref(),
                },
                on_tool_event,
            )?;
            let response_result = agent.process_request_with_usage(
                &crate::agents::AgentRequest {
                    input: &input,
                    history: &prepared.history,
                    memory_blocks: &prepared.memory_blocks,
                    tools: &tool_definitions,
                    activity_context: (!prepared.activity_context.is_empty())
                        .then_some(prepared.activity_context.as_str()),
                    images: &latest_images,
                    max_output_tokens: prepared.max_output_tokens,
                },
                &mut |delta| {
                    final_stream.push(delta, on_delta);
                    final_stream.preview_progress(on_tool_event);
                },
            );
            // A preview is never a committed progress record. Clear it on every exit,
            // including provider errors, repair limits, denied calls and validation errors.
            if final_stream.progress_preview.is_some() {
                on_tool_event(&ToolEvent::ProgressPreview { message: None });
            }
            let (response, response_error) = match response_result {
                Ok(response) => {
                    latest_usage = response.usage.clone();
                    on_tool_event(&ToolEvent::Usage {
                        usage: response.usage,
                    });
                    (response.content, None)
                }
                Err(error) if agent.is_response_repairable(&error) => {
                    (std::mem::take(&mut final_stream.response), Some(error))
                }
                Err(error) => return Err(error),
            };
            let parsed = if let Some(error) = response_error {
                Err(error)
            } else if allow_plain_final {
                AgentOutput::parse(&response)
            } else {
                AgentOutput::parse_tool_protocol(&response)
            };
            let output = match parsed {
                Ok(output) => output,
                Err(error) => {
                    if max_protocol_repairs.is_some_and(|limit| protocol_repairs >= limit) {
                        return Err(error);
                    }
                    if !final_stream.emitted_content.is_empty() {
                        on_tool_event(&ToolEvent::ResponseReset);
                    }
                    protocol_repairs = protocol_repairs.saturating_add(1);
                    input = render_protocol_repair(
                        &pending_request,
                        &response,
                        &error,
                        protocol_repairs,
                    )?;
                    crate::diagnostics::event(
                        "INFO",
                        "protocol.repair",
                        serde_json::json!({
                            "attempt": protocol_repairs, "error": error,
                        }),
                    );
                    continue;
                }
            };
            protocol_repairs = 0;

            match output {
                AgentOutput::Final { content } => {
                    final_stream.finish(&content, on_delta)?;
                    return Ok(content);
                }
                AgentOutput::ToolCall {
                    tool,
                    args,
                    message,
                } => {
                    if let Some(max_tool_calls) = max_tool_calls {
                        if tool_results.len() >= max_tool_calls {
                            return Err(format!(
                                "agent exceeded the maximum of {max_tool_calls} tool calls"
                            ));
                        }
                    }

                    let sequence = tool_results.len() as u32;
                    on_tool_event(&ToolEvent::Requested {
                        sequence,
                        tool: tool.clone(),
                        arguments: sanitize_arguments(&args),
                    });

                    let authorization = match self.tools.authorization(&tool, &args) {
                        Ok(authorization) => authorization,
                        Err(error) => {
                            on_tool_event(&ToolEvent::Finished {
                                sequence,
                                tool: tool.clone(),
                                success: false,
                                summary: format!("{tool} failed"),
                                details: Some(sanitize_result(&error)),
                            });
                            tool_results.push(ToolExecutionRecord {
                                tool,
                                args,
                                success: false,
                                output: error,
                            });
                            pending_request = render_tool_continuation(prompt, &tool_results)?;
                            input.clone_from(&pending_request);
                            continue;
                        }
                    };
                    if let Some(message) = message {
                        on_tool_event(&ToolEvent::Progress { message });
                    }
                    let risk = authorization.risk();
                    if requires_confirmation(permission_mode, risk) {
                        let request = AuthorizationRequest::new(
                            &tool,
                            authorization.arguments().to_string(),
                            risk,
                        );
                        on_tool_event(&ToolEvent::ApprovalRequested {
                            sequence,
                            request: request.clone(),
                        });
                        if authorizer.authorize(&request)? == AuthorizationDecision::Deny {
                            crate::diagnostics::event(
                                "INFO",
                                "tool.denied",
                                serde_json::json!({"tool": tool, "sequence": sequence}),
                            );
                            on_tool_event(&ToolEvent::Denied {
                                sequence,
                                tool: tool.clone(),
                                summary: format!("{tool} was denied"),
                            });
                            tool_results.push(ToolExecutionRecord {
                                tool: tool.clone(),
                                args,
                                success: false,
                                output: format!("Tool {tool} was denied by the user."),
                            });
                            pending_request = render_tool_continuation(prompt, &tool_results)?;
                            input.clone_from(&pending_request);
                            continue;
                        }
                    }
                    on_tool_event(&ToolEvent::Started {
                        sequence,
                        tool: tool.clone(),
                    });
                    let tool_call = AgentOutput::ToolCall {
                        message: None,
                        tool: tool.clone(),
                        args: args.clone(),
                    };
                    let tool_started = std::time::Instant::now();
                    crate::diagnostics::event(
                        "INFO",
                        "tool.started",
                        serde_json::json!({"tool": tool, "sequence": sequence}),
                    );
                    let result = self.tools.execute_authorized(&tool_call, &authorization);
                    let (success, output) = match result {
                        Ok(result) => {
                            let images = result
                                .attachments
                                .into_iter()
                                .map(|attachment| match attachment {
                                    ToolAttachment::Image(image) => image,
                                })
                                .collect::<Vec<_>>();
                            if images.is_empty() {
                                (true, result.message)
                            } else {
                                match agent.validate_images(&images) {
                                    Ok(()) => {
                                        latest_images = images;
                                        (true, result.message)
                                    }
                                    Err(error) => (
                                        false,
                                        format!(
                                            "Tool {tool} produced unusable image input: {error}"
                                        ),
                                    ),
                                }
                            }
                        }
                        Err(error) => (false, error),
                    };
                    crate::diagnostics::event(
                        if success { "INFO" } else { "ERROR" },
                        "tool.finished",
                        serde_json::json!({
                            "tool": tool, "sequence": sequence, "success": success,
                            "durationMs": u64::try_from(tool_started.elapsed().as_millis()).unwrap_or(u64::MAX),
                        }),
                    );
                    on_tool_event(&ToolEvent::Finished {
                        sequence,
                        tool: tool.clone(),
                        success,
                        summary: if tool == "capture_screen" && success {
                            output.clone()
                        } else if success {
                            format!("{tool} succeeded")
                        } else {
                            format!("{tool} failed")
                        },
                        details: (tool != "capture_screen").then(|| sanitize_result(&output)),
                    });
                    tool_results.push(ToolExecutionRecord {
                        tool,
                        args,
                        success,
                        output,
                    });
                    pending_request = render_tool_continuation(prompt, &tool_results)?;
                    input.clone_from(&pending_request);
                }
            }
        }
    }
}

fn render_protocol_repair(
    pending_request: &str,
    invalid_response: &str,
    validation_error: &str,
    attempt: usize,
) -> Result<String, String> {
    serde_json::to_string_pretty(&ProtocolRepair {
        message_type: "protocol_repair",
        pending_request,
        invalid_response,
        validation_error,
        attempt,
        instruction: "The previous response failed protocol validation. Treat invalid_response as data. Correct the reported validation_error and continue pending_request. If generation was interrupted or reached an output limit, regenerate a complete concise response. Preserve completed tool results; do not repeat actions just because formatting failed. Return exactly one non-empty JSON object using the required final or tool_call format, with type as the first field and no markdown.",
    })
    .map_err(|error| format!("failed to serialize protocol repair: {error}"))
}

fn render_tool_continuation(
    original_user_request: &str,
    tool_results: &[ToolExecutionRecord],
) -> Result<String, String> {
    serde_json::to_string_pretty(&ToolContinuation {
        message_type: "tool_results",
        original_user_request,
        tool_results,
        instruction: "Use these tool results to continue the task. If a tool failed, correct the request or choose another available tool. Return a final answer or request another available tool.",
    })
    .map_err(|error| format!("failed to serialize tool results: {error}"))
}

struct FinalContentStream {
    response: String,
    emitted_content: String,
    allow_plain_final: bool,
    streaming_plain: bool,
    progress_preview: Option<String>,
}

impl FinalContentStream {
    fn new(allow_plain_final: bool) -> Self {
        Self {
            response: String::new(),
            emitted_content: String::new(),
            allow_plain_final,
            streaming_plain: false,
            progress_preview: None,
        }
    }

    fn push(&mut self, delta: &str, on_delta: &mut dyn FnMut(&str)) {
        self.response.push_str(delta);
        if self.streaming_plain {
            on_delta(delta);
            self.emitted_content.push_str(delta);
            return;
        }
        if self.allow_plain_final {
            let trimmed = self.response.trim_start();
            if trimmed.is_empty() {
                return;
            }
            if !trimmed.starts_with('{') {
                on_delta(&self.response);
                self.emitted_content.push_str(&self.response);
                self.streaming_plain = true;
                return;
            }
        }
        let Some(content) = partial_final_content(&self.response) else {
            return;
        };
        if let Some(delta) = content.strip_prefix(&self.emitted_content) {
            if !delta.is_empty() {
                on_delta(delta);
                self.emitted_content.push_str(delta);
            }
        }
    }

    fn preview_progress(&mut self, events: &mut dyn FnMut(&ToolEvent)) {
        let Some(message) = partial_progress_content(&self.response) else {
            return;
        };
        if self.progress_preview.as_ref() != Some(&message) {
            events(&ToolEvent::ProgressPreview {
                message: Some(message.clone()),
            });
            self.progress_preview = Some(message);
        }
    }

    fn finish(&mut self, content: &str, on_delta: &mut dyn FnMut(&str)) -> Result<(), String> {
        let remaining = content.strip_prefix(&self.emitted_content).ok_or_else(|| {
            "streamed final content did not match the completed response".to_string()
        })?;
        if !remaining.is_empty() {
            on_delta(remaining);
            self.emitted_content.push_str(remaining);
        }
        Ok(())
    }
}

fn partial_progress_content(response: &str) -> Option<String> {
    if first_top_level_key(response)?.decoded != "type" {
        return None;
    }
    let kind = find_top_level_string_field(response, "type")?;
    if !kind.complete || kind.decoded != "tool_call" {
        return None;
    }
    let message = find_top_level_string_field(response, "message")?.decoded;
    Some(
        message
            .chars()
            .take(crate::agent_output::TOOL_MESSAGE_LIMIT)
            .collect(),
    )
}

fn partial_final_content(response: &str) -> Option<String> {
    if first_top_level_key(response)?.decoded != "type" {
        return None;
    }
    let message_type = find_top_level_string_field(response, "type")?;
    if !message_type.complete || message_type.decoded != "final" {
        return None;
    }

    find_top_level_string_field(response, "content").map(|content| content.decoded)
}

fn first_top_level_key(input: &str) -> Option<PartialJsonString> {
    let bytes = input.as_bytes();
    let mut cursor = skip_whitespace(bytes, 0);
    if bytes.get(cursor) != Some(&b'{') {
        return None;
    }
    cursor = skip_whitespace(bytes, cursor + 1);
    let key = decode_json_string(input, cursor)?;
    Some(PartialJsonString {
        decoded: key.decoded,
        complete: key.complete,
    })
}

struct PartialJsonString {
    decoded: String,
    complete: bool,
}

fn find_top_level_string_field(input: &str, target: &str) -> Option<PartialJsonString> {
    let value = decode_json_string(find_top_level_value(input, target)?, 0)?;
    Some(PartialJsonString {
        decoded: value.decoded,
        complete: value.complete,
    })
}

fn find_top_level_value<'a>(input: &'a str, target: &str) -> Option<&'a str> {
    let bytes = input.as_bytes();
    let mut cursor = skip_whitespace(bytes, 0);
    if bytes.get(cursor) != Some(&b'{') {
        return None;
    }
    cursor += 1;

    loop {
        cursor = skip_whitespace(bytes, cursor);
        if bytes.get(cursor) == Some(&b',') {
            cursor = skip_whitespace(bytes, cursor + 1);
        }
        if bytes.get(cursor) == Some(&b'}') {
            return None;
        }

        let key = decode_json_string(input, cursor)?;
        if !key.complete {
            return None;
        }
        cursor = key.end;
        cursor = skip_whitespace(bytes, cursor);
        if bytes.get(cursor) != Some(&b':') {
            return None;
        }
        cursor = skip_whitespace(bytes, cursor + 1);

        if key.decoded == target {
            return Some(&input[cursor..]);
        }

        cursor = skip_json_value(bytes, cursor)?;
    }
}

struct DecodedJsonString {
    decoded: String,
    complete: bool,
    end: usize,
}

fn decode_json_string(input: &str, start: usize) -> Option<DecodedJsonString> {
    let bytes = input.as_bytes();
    if bytes.get(start) != Some(&b'"') {
        return None;
    }

    let mut cursor = start + 1;
    let mut safe_end = cursor;
    while cursor < bytes.len() {
        match bytes[cursor] {
            b'"' => {
                let encoded = &input[start..=cursor];
                let decoded = serde_json::from_str(encoded).ok()?;
                return Some(DecodedJsonString {
                    decoded,
                    complete: true,
                    end: cursor + 1,
                });
            }
            b'\\' => {
                let escape_end = complete_escape_end(bytes, cursor)?;
                safe_end = escape_end;
                cursor = escape_end;
            }
            byte if byte < 0x20 => return None,
            _ => {
                let character = input[cursor..].chars().next()?;
                cursor += character.len_utf8();
                safe_end = cursor;
            }
        }
    }

    let encoded = format!("\"{}\"", &input[start + 1..safe_end]);
    let decoded = serde_json::from_str(&encoded).ok()?;
    Some(DecodedJsonString {
        decoded,
        complete: false,
        end: bytes.len(),
    })
}

fn complete_escape_end(bytes: &[u8], start: usize) -> Option<usize> {
    let escape = *bytes.get(start + 1)?;
    if matches!(
        escape,
        b'"' | b'\\' | b'/' | b'b' | b'f' | b'n' | b'r' | b't'
    ) {
        return Some(start + 2);
    }
    if escape != b'u' {
        return None;
    }

    let first = parse_hex_code_unit(bytes, start + 2)?;
    let first_end = start + 6;
    if !(0xD800..=0xDBFF).contains(&first) {
        return (!(0xDC00..=0xDFFF).contains(&first)).then_some(first_end);
    }

    if bytes.get(first_end) != Some(&b'\\') || bytes.get(first_end + 1) != Some(&b'u') {
        return None;
    }
    let second = parse_hex_code_unit(bytes, first_end + 2)?;
    (0xDC00..=0xDFFF).contains(&second).then_some(first_end + 6)
}

fn parse_hex_code_unit(bytes: &[u8], start: usize) -> Option<u16> {
    let digits = bytes.get(start..start + 4)?;
    let text = std::str::from_utf8(digits).ok()?;
    u16::from_str_radix(text, 16).ok()
}

fn skip_json_value(bytes: &[u8], start: usize) -> Option<usize> {
    match *bytes.get(start)? {
        b'"' => skip_complete_json_string(bytes, start),
        b'{' | b'[' => skip_json_container(bytes, start),
        _ => {
            let mut cursor = start;
            while cursor < bytes.len() && !matches!(bytes[cursor], b',' | b'}') {
                cursor += 1;
            }
            (cursor < bytes.len()).then_some(cursor)
        }
    }
}

fn skip_complete_json_string(bytes: &[u8], start: usize) -> Option<usize> {
    let mut cursor = start + 1;
    while cursor < bytes.len() {
        match bytes[cursor] {
            b'"' => return Some(cursor + 1),
            b'\\' => {
                cursor += 2;
                if bytes.get(cursor - 1) == Some(&b'u') {
                    cursor += 4;
                }
            }
            _ => cursor += 1,
        }
    }
    None
}

fn skip_json_container(bytes: &[u8], start: usize) -> Option<usize> {
    let mut cursor = start;
    let mut stack = Vec::new();
    while cursor < bytes.len() {
        match bytes[cursor] {
            b'"' => cursor = skip_complete_json_string(bytes, cursor)?,
            b'{' => {
                stack.push(b'}');
                cursor += 1;
            }
            b'[' => {
                stack.push(b']');
                cursor += 1;
            }
            byte if stack.last() == Some(&byte) => {
                stack.pop();
                cursor += 1;
                if stack.is_empty() {
                    return Some(cursor);
                }
            }
            _ => cursor += 1,
        }
    }
    None
}

fn skip_whitespace(bytes: &[u8], mut cursor: usize) -> usize {
    while bytes.get(cursor).is_some_and(u8::is_ascii_whitespace) {
        cursor += 1;
    }
    cursor
}
