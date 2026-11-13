pub mod context;
mod wakeup;
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
    pub(crate) execution_scope: crate::executions::ExecutionScope,
    pub(crate) delegation: Option<crate::delegation::Delegation>,
    pub(crate) run_control: Option<crate::agent_runs::RunControl>,
    #[cfg(any(unix, windows))]
    pub(crate) owned_process_scope: Option<crate::tools::ProcessScope>,
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
            execution_scope: crate::executions::ToolExecutionManager::new()
                .scope(&uuid::Uuid::new_v4().to_string()),
            delegation: None,
            run_control: None,
            #[cfg(any(unix, windows))]
            owned_process_scope: None,
        }
    }

    pub fn with_execution_scope(mut self, scope: crate::executions::ExecutionScope) -> Self {
        if let Some(delegation) = &mut self.delegation {
            delegation.set_execution_scope(scope.clone());
        }
        self.execution_scope = scope;
        self
    }

    pub fn with_tools(mut self, tools: ToolRegistry) -> Self {
        self.tools = tools;
        self
    }

    #[cfg(any(unix, windows))]
    pub fn with_process_scope(
        mut self,
        workspace: &std::path::Path,
        scope: crate::tools::ProcessScope,
    ) -> Self {
        self.owned_process_scope = None;
        crate::tools::register_process_tools_in_scope(&mut self.tools, workspace, scope.clone());
        if let Some(delegation) = &mut self.delegation {
            delegation.set_process_scope(scope);
        }
        self
    }
    #[cfg(any(unix, windows))]
    pub(crate) fn with_request_process_scope(
        mut self,
        workspace: &std::path::Path,
        scope: crate::tools::ProcessScope,
        managed: bool,
    ) -> Self {
        crate::tools::register_request_process_tools(
            &mut self.tools,
            workspace,
            scope.clone(),
            managed,
        );
        if let Some(delegation) = &mut self.delegation {
            delegation.set_request_process_scope(scope);
        }
        self
    }

    pub fn with_delegation(mut self, mut delegation: crate::delegation::Delegation) -> Self {
        delegation.set_execution_scope(self.execution_scope.clone());
        crate::delegation::register_tools(&mut self.tools);
        self.delegation = Some(delegation);
        self
    }

    pub fn with_agent_catalog(mut self, catalog: crate::agent_catalog::AgentCatalog) -> Self {
        crate::tools::register_agent_catalog_tools(&mut self.tools, catalog);
        self
    }

    pub(crate) fn select_skills(
        mut self,
        store: &crate::skills::SkillStore,
        ids: &[String],
    ) -> Result<Self, String> {
        self.tools.remove("load_skill");
        if let Some(tool) = crate::tools::LoadSkillTool::selected(store, ids)? {
            self.tools.register(crate::executions::worker::tool(tool));
        }
        Ok(self)
    }

    pub fn restrict_tools(mut self, allowed: &[String]) -> Self {
        self.tools.retain_names(allowed);
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
        let _execution_consumer = self.execution_scope.acquire_consumer()?;
        let _tool_request_scope = self.tools.request_scope();
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
        let mut wakeup_batch: Option<crate::executions::WakeupBatch> = None;
        let mut control_result: Option<serde_json::Value> = None;
        let mut owned_executions = std::collections::HashMap::<String, (u32, String)>::new();
        let mut jobs = wakeup::JobsGuard::new(self.execution_scope.clone());
        let mut next_sequence = 0u32;
        let mut has_wakeup_controls = false;

        loop {
            self.execution_scope.check_open()?;
            if protocol_repairs == 0 {
                if wakeup_batch.is_none() {
                    wakeup_batch = self.execution_scope.claim()?;
                }
                for info in self.execution_scope.list()? {
                    if matches!(info.status.as_str(), "succeeded" | "failed" | "cancelled") {
                        jobs.remove(&info.execution_id);
                        if let Some((sequence, tool)) = owned_executions.remove(&info.execution_id)
                        {
                            on_tool_event(&ToolEvent::Finished {
                                sequence,
                                tool: tool.clone(),
                                success: info.status == "succeeded",
                                summary: format!("{tool} {}", info.status),
                                details: Some(
                                    serde_json::to_string(&info)
                                        .map_err(|_| "Could not encode execution status")?,
                                ),
                            });
                        }
                    }
                }
                let infos = self.execution_scope.list()?;
                if !infos.is_empty()
                    || has_wakeup_controls
                    || wakeup_batch.is_some()
                    || control_result.is_some()
                {
                    pending_request = wakeup::continuation(
                        prompt,
                        &tool_results,
                        wakeup_batch.as_ref(),
                        control_result.as_ref(),
                        &infos,
                    )?;
                    input.clone_from(&pending_request);
                }
            }
            if let Some(control) = &self.run_control {
                control.check()?;
            }
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
            if let Some(control) = &self.run_control {
                control.check()?;
            }
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
            self.execution_scope.check_open()?;
            protocol_repairs = 0;
            if let Some(batch) = wakeup_batch.take() {
                self.execution_scope.ack(&batch.batch_id)?;
            }
            control_result = None;
            let (output, execution_policy) = match output {
                AgentOutput::ScheduledToolCall {
                    tool,
                    mut args,
                    message,
                    execution,
                } => {
                    has_wakeup_controls = true;
                    if tool == "exec_command" {
                        if let Some(args) = args.as_object_mut() {
                            if args
                                .get("yield_time_ms")
                                .is_none_or(serde_json::Value::is_null)
                                && args.get("lifetime").and_then(|v| v.as_str())
                                    != Some("background")
                            {
                                args.insert("yield_time_ms".into(), serde_json::json!(0));
                                if args
                                    .get("timeout_ms")
                                    .is_none_or(serde_json::Value::is_null)
                                {
                                    args.insert("timeout_ms".into(), serde_json::json!(30000));
                                }
                            }
                        }
                    }
                    if tool == "call_agent" {
                        if let Some(args) = args.as_object_mut() {
                            args.insert("execution_mode".into(), serde_json::json!("parallel"));
                        }
                    }
                    (
                        AgentOutput::ToolCall {
                            tool,
                            args,
                            message,
                        },
                        Some(execution),
                    )
                }
                AgentOutput::Execution { action } => {
                    has_wakeup_controls = true;
                    if max_tool_calls.is_some_and(|limit| next_sequence as usize >= limit) {
                        return Err("agent exceeded its maximum tool/control calls".into());
                    }
                    let sequence = next_sequence;
                    next_sequence = next_sequence.saturating_add(1);
                    let name = match &action {
                        crate::executions::protocol::ExecutionAction::Wait { .. } => {
                            "await_execution"
                        }
                        crate::executions::protocol::ExecutionAction::Read { .. } => {
                            "read_execution"
                        }
                        crate::executions::protocol::ExecutionAction::List => "list_executions",
                        crate::executions::protocol::ExecutionAction::Cancel { .. } => {
                            "cancel_execution"
                        }
                    };
                    let control_args = action.arguments();
                    on_tool_event(&ToolEvent::Requested {
                        sequence,
                        tool: name.into(),
                        arguments: sanitize_arguments(&control_args),
                    });
                    on_tool_event(&ToolEvent::Started {
                        sequence,
                        tool: name.into(),
                    });
                    let is_wait = matches!(
                        &action,
                        crate::executions::protocol::ExecutionAction::Wait { .. }
                    );
                    let outcome = self.execution_control(
                        action,
                        authorizer,
                        on_tool_event,
                        permission_mode,
                        sequence,
                        &mut latest_images,
                    );
                    let success = outcome.is_ok();
                    match outcome {
                        Ok(value) if is_wait => {
                            // Wait claims an immutable batch; acknowledge only after a valid model decision.
                            wakeup_batch = self.execution_scope.claim()?;
                            let _ = value;
                            control_result = None;
                        }
                        Ok(value) => {
                            control_result =
                                Some(serde_json::json!({"success":true,"result":value}))
                        }
                        Err(error) => {
                            control_result =
                                Some(serde_json::json!({"success":false,"error":error}))
                        }
                    }
                    on_tool_event(&ToolEvent::Finished {
                        sequence,
                        tool: name.into(),
                        success,
                        summary: format!("{name} {}", if success { "succeeded" } else { "failed" }),
                        details: control_result
                            .as_ref()
                            .map(|v| sanitize_result(&v.to_string())),
                    });
                    if !is_wait {
                        let value = control_result.take().unwrap_or(serde_json::Value::Null);
                        tool_results.push(ToolExecutionRecord {
                            tool: name.into(),
                            args: control_args,
                            success,
                            output: value.to_string(),
                        });
                        let mut bytes = 0;
                        for record in tool_results
                            .iter_mut()
                            .rev()
                            .filter(|r| r.tool == "read_execution")
                        {
                            bytes += record.output.len();
                            if bytes > 32768 {
                                record.output="{\"notice\":\"Older selected output omitted; use read_execution again if needed\"}".into();
                            }
                        }
                    }
                    continue;
                }
                other => (other, None),
            };
            match output {
                AgentOutput::ScheduledToolCall { .. } | AgentOutput::Execution { .. } => {
                    unreachable!("Wakeup protocol normalized above")
                }
                AgentOutput::Final { content } => {
                    if let Some(batch) = self.execution_scope.claim()? {
                        if !final_stream.emitted_content.is_empty() {
                            on_tool_event(&ToolEvent::ResponseReset);
                        }
                        wakeup_batch = Some(batch);
                        continue;
                    }
                    if !jobs.is_empty() {
                        if !final_stream.emitted_content.is_empty() {
                            on_tool_event(&ToolEvent::ResponseReset);
                        }
                        wakeup_batch = Some(self.wait_executions(
                            &jobs.iter().cloned().collect::<Vec<_>>(),
                            2000,
                            authorizer,
                            on_tool_event,
                        )?);
                        continue;
                    }
                    if let Some(control) = &self.run_control {
                        control.check()?;
                    }
                    final_stream.finish(&content, on_delta)?;
                    return Ok(content);
                }
                AgentOutput::ToolCall {
                    tool,
                    args,
                    message,
                } => {
                    if let Some(max_tool_calls) = max_tool_calls {
                        if next_sequence as usize >= max_tool_calls {
                            return Err(format!(
                                "agent exceeded the maximum of {max_tool_calls} tool calls"
                            ));
                        }
                    }

                    let sequence = next_sequence;
                    next_sequence = next_sequence.saturating_add(1);
                    on_tool_event(&ToolEvent::Requested {
                        sequence,
                        tool: tool.clone(),
                        arguments: sanitize_arguments(&args),
                    });

                    let child_read_error = self.run_control.is_some()
                        && tool == "read_process"
                        && args.get("cursor").is_none_or(serde_json::Value::is_null)
                        && args
                            .get("tail_lines")
                            .is_none_or(serde_json::Value::is_null);
                    let authorization_result = if child_read_error {
                        Err("Child reads require an explicit cursor or tail_lines; default chat cursor is reserved for the parent".into())
                    } else {
                        self.tools.authorization(&tool, &args)
                    };
                    let authorization = match authorization_result {
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
                    self.execution_scope.check_open()?;
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
                    if let Some(control) = &self.run_control {
                        control.check()?;
                    }
                    if let Some(policy) = execution_policy {
                        let start = self.execution_scope.reserve(&tool).and_then(|slot| {
                            let preparation =
                                std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                                    let work = if crate::delegation::is_run_tool(&tool) {
                                        match &self.delegation {
                                            Some(delegation) => delegation.prepare_wakeup(
                                                &tool,
                                                &args,
                                                crate::delegation::CallContext {
                                                    prepared: &prepared,
                                                    parent_input: &pending_request,
                                                    available_tools: &tool_definitions,
                                                    permission_mode,
                                                    authorizer,
                                                    on_tool_event,
                                                    sequence,
                                                    control: self.run_control.as_ref(),
                                                },
                                            ),
                                            None => Err("Agent delegation is unavailable".into()),
                                        }
                                    } else {
                                        self.tools.prepare_execution(authorization)
                                    };
                                    work
                                }))
                                .unwrap_or_else(|_| {
                                    Err("Tool preparation panicked; action outcome may be unknown"
                                        .into())
                                });
                            match preparation {
                                Ok(work) => self.execution_scope.launch(&slot.execution_id, work),
                                Err(error) => {
                                    self.execution_scope
                                        .fail_reserved(&slot.execution_id, error.clone());
                                    Err(error)
                                }
                            }
                        });
                        match start {
                            Ok(info) => {
                                if policy.mode == crate::executions::protocol::ExecutionMode::Job
                                    || tool == "exec_command"
                                        && args.get("lifetime").and_then(|v| v.as_str())
                                            != Some("background")
                                {
                                    jobs.insert(info.execution_id.clone());
                                }
                                owned_executions
                                    .insert(info.execution_id.clone(), (sequence, tool.clone()));
                                tool_results.push(ToolExecutionRecord {
                                    tool,
                                    args,
                                    success: true,
                                    output: serde_json::to_string(&info)
                                        .map_err(|_| "Could not encode tool execution")?,
                                });
                                if policy.mode == crate::executions::protocol::ExecutionMode::Job {
                                    wakeup_batch = Some(self.wait_executions(
                                        &[info.execution_id],
                                        policy.yield_ms,
                                        authorizer,
                                        on_tool_event,
                                    )?);
                                }
                            }
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
                            }
                        }
                        continue;
                    }
                    let result = if crate::delegation::is_run_tool(&tool) {
                        match &self.delegation {
                            Some(delegation) => delegation
                                .execute(
                                    &tool,
                                    &args,
                                    crate::delegation::CallContext {
                                        prepared: &prepared,
                                        parent_input: &pending_request,
                                        available_tools: &tool_definitions,
                                        permission_mode,
                                        authorizer,
                                        on_tool_event,
                                        sequence,
                                        control: self.run_control.as_ref(),
                                    },
                                )
                                .map(crate::tools::ToolResult::text),
                            None => Err("Agent delegation is unavailable".into()),
                        }
                    } else {
                        self.tools.execute_authorized(&tool_call, &authorization)
                    };
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

impl Drop for Runtime {
    fn drop(&mut self) {
        #[cfg(any(unix, windows))]
        if let Some(scope) = &self.owned_process_scope {
            if let Some(delegation) = &self.delegation {
                delegation.close_owner();
            }
            let _ = scope.close(std::time::Duration::from_secs(3));
        }
    }
}
