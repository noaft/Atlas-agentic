use serde::Serialize;

use crate::{
    agent_output::AgentOutput,
    agents::AgentRegistry,
    conversations::ConversationMessage,
    media::ImageData,
    permissions::{
        requires_confirmation, AuthorizationDecision, AuthorizationRequest, MissingAuthorizer,
        PermissionMode, ToolAuthorizer, ToolEvent,
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
    instruction: &'static str,
}

pub struct Runtime {
    agents: AgentRegistry,
    tools: ToolRegistry,
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
        let agent = self
            .agents
            .default_agent()
            .ok_or_else(|| "no agent is registered".to_string())?;
        let tool_definitions = self.tools.definitions();
        let max_tool_calls = agent.max_tool_calls();

        let mut input = prompt.to_string();
        let mut tool_results = Vec::new();
        let mut latest_images: Vec<ImageData> = Vec::new();
        let mut retried_invalid_response = false;
        let allow_plain_final = tool_definitions.is_empty();

        loop {
            let mut final_stream = FinalContentStream::new(allow_plain_final);
            let response = agent.process_stream_with_images(
                &input,
                history,
                &tool_definitions,
                &latest_images,
                &mut |delta| {
                    final_stream.push(delta, on_delta);
                },
            )?;
            let parsed = if allow_plain_final {
                AgentOutput::parse(&response)
            } else {
                AgentOutput::parse_tool_protocol(&response)
            };
            let output = match parsed {
                Ok(output) => output,
                Err(_) if !retried_invalid_response => {
                    input = render_protocol_repair(&input)?;
                    retried_invalid_response = true;
                    continue;
                }
                Err(error) => return Err(error),
            };
            retried_invalid_response = false;

            match output {
                AgentOutput::Final { content } => {
                    final_stream.finish(&content, on_delta)?;
                    return Ok(content);
                }
                AgentOutput::ToolCall { tool, args } => {
                    if let Some(max_tool_calls) = max_tool_calls {
                        if tool_results.len() >= max_tool_calls {
                            return Err(format!(
                                "agent exceeded the maximum of {max_tool_calls} tool calls"
                            ));
                        }
                    }

                    let (risk, arguments) = match self.tools.authorization(&tool, &args) {
                        Ok(authorization) => authorization,
                        Err(error) => {
                            tool_results.push(ToolExecutionRecord {
                                tool,
                                args,
                                success: false,
                                output: error,
                            });
                            input = render_tool_continuation(prompt, &tool_results)?;
                            continue;
                        }
                    };
                    if requires_confirmation(permission_mode, risk) {
                        let request = AuthorizationRequest::new(&tool, arguments, risk);
                        on_tool_event(&ToolEvent::ApprovalRequested {
                            request: request.clone(),
                        });
                        if authorizer.authorize(&request)? == AuthorizationDecision::Deny {
                            tool_results.push(ToolExecutionRecord {
                                tool: tool.clone(),
                                args,
                                success: false,
                                output: format!("Tool {tool} was denied by the user."),
                            });
                            input = render_tool_continuation(prompt, &tool_results)?;
                            continue;
                        }
                    }
                    on_tool_event(&ToolEvent::Started { tool: tool.clone() });
                    let tool_call = AgentOutput::ToolCall {
                        tool: tool.clone(),
                        args: args.clone(),
                    };
                    let result = self.tools.execute(&tool_call);
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
                    on_tool_event(&ToolEvent::Finished {
                        tool: tool.clone(),
                        success,
                    });
                    tool_results.push(ToolExecutionRecord {
                        tool,
                        args,
                        success,
                        output,
                    });
                    input = render_tool_continuation(prompt, &tool_results)?;
                }
            }
        }
    }
}

fn render_protocol_repair(pending_request: &str) -> Result<String, String> {
    serde_json::to_string_pretty(&ProtocolRepair {
        message_type: "protocol_repair",
        pending_request,
        instruction: "The previous response did not follow the required tool protocol. Return exactly one non-empty JSON object using the required final or tool_call format, with type as the first field and no markdown.",
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
}

impl FinalContentStream {
    fn new(allow_plain_final: bool) -> Self {
        Self {
            response: String::new(),
            emitted_content: String::new(),
            allow_plain_final,
            streaming_plain: false,
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
            let value = decode_json_string(input, cursor)?;
            return Some(PartialJsonString {
                decoded: value.decoded,
                complete: value.complete,
            });
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
