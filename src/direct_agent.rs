//! Native, one-turn routing to a saved agent without an Atlas model call.
use crate::{
    agent_runs::{AgentRunRequest, AgentRunStatus, ContextMode, ExecutionMode},
    delegation::CallContext,
    permissions::{PermissionMode, ToolEvent},
    runtime::context::{ContextPreparationRequest, RuntimeContext},
    Runtime, RuntimeAskCallbacks,
};

pub(crate) struct DirectCommand {
    agent: String,
    task: String,
}

impl Runtime {
    pub fn is_direct_agent(&self) -> bool {
        self.direct_agent.is_some()
    }

    /// Resolve only at a native request boundary, after workspace and tools are known.
    /// This affects this runtime instance only; the next ordinary request uses Atlas.
    pub fn with_direct_agent_command(mut self, prompt: &str) -> Result<Self, String> {
        self.direct_agent = None;
        let Some(command) = prompt.trim_start().strip_prefix('/') else {
            return Ok(self);
        };
        let boundary = command.find(char::is_whitespace).unwrap_or(command.len());
        let name = &command[..boundary];
        let task = command[boundary..].trim();
        if task.is_empty() {
            return Err("Provide a task after /agent-name".into());
        }
        let delegation = self
            .delegation
            .as_ref()
            .ok_or("Agent dispatch is unavailable")?;
        let agent = delegation.resolve_command(name)?;
        if task.len() > 65536 {
            return Err("Agent task exceeds 64 KiB".into());
        }
        self.direct_agent = Some(DirectCommand {
            agent: agent.id,
            task: task.into(),
        });
        Ok(self)
    }

    pub(crate) fn ask_direct_agent(
        &self,
        command: &DirectCommand,
        context: &mut dyn RuntimeContext,
        permission_mode: PermissionMode,
        callbacks: RuntimeAskCallbacks<'_>,
    ) -> Result<String, String> {
        let delegation = self
            .delegation
            .as_ref()
            .ok_or("Agent dispatch is unavailable")?;
        if let Some(control) = &self.turn_control {
            if !control.try_finish() {
                return Err("Direct agent dispatch cannot accept supplemental input".into());
            }
            (callbacks.on_tool_event)(&ToolEvent::RequestStarted {
                request_id: control.request_id().into(),
            });
        }
        // Keep new messages queued in the UI for the next turn, rather than
        // forwarding them into a parent's inbox that this direct run does not use.
        (callbacks.on_tool_event)(&ToolEvent::ModelState { generating: true });
        self.check_direct_cancelled(context)?;
        let tools = self.tool_definitions();
        let prepared = context.prepare(
            self.default_agent()?,
            ContextPreparationRequest {
                input: &command.task,
                tools: &tools,
                images: &[],
                latest_usage: None,
            },
            callbacks.on_tool_event,
        )?;
        self.check_direct_cancelled(context)?;
        let request = AgentRunRequest {
            agent: command.agent.clone(),
            task: command.task.clone(),
            tools: tools.iter().map(|tool| tool.name.clone()).collect(),
            context_mode: ContextMode::Fork,
            execution_mode: ExecutionMode::Parallel,
        };
        let arguments =
            serde_json::to_string(&request).map_err(|_| "Invalid direct agent request")?;
        (callbacks.on_tool_event)(&ToolEvent::Requested {
            sequence: 0,
            tool: "call_agent".into(),
            arguments,
        });
        (callbacks.on_tool_event)(&ToolEvent::Started {
            sequence: 0,
            tool: "call_agent".into(),
        });
        self.check_direct_cancelled(context)?;
        let started = delegation.start(
            request,
            &CallContext {
                prepared: &prepared,
                parent_input: &command.task,
                available_tools: &tools,
                permission_mode,
                authorizer: callbacks.authorizer,
                on_tool_event: callbacks.on_tool_event,
                sequence: 0,
                control: None,
            },
        );
        let (started, _) = match started {
            Ok(started) => started,
            Err(error) => {
                (callbacks.on_tool_event)(&ToolEvent::Finished {
                    sequence: 0,
                    tool: "call_agent".into(),
                    success: false,
                    summary: format!("{} could not start", command.agent),
                    details: Some(error.clone()),
                });
                return Err(error);
            }
        };
        let mut guard = DirectRunGuard {
            delegation,
            id: &started.run_id,
        };
        let mut cancellation = None;
        let completed = loop {
            if cancellation.is_none() {
                if let Err(error) = self.check_direct_cancelled(context) {
                    delegation.cancel_direct(&started.run_id)?;
                    cancellation = Some(error);
                }
            }
            // Native desktop approvals remain on the child's own inline card.
            if cancellation.is_none() {
                if let Err(error) = delegation.service_wakeup_approvals(
                    callbacks.authorizer,
                    callbacks.on_tool_event,
                    false,
                ) {
                    delegation.cancel_direct(&started.run_id)?;
                    cancellation = Some(error);
                }
            }
            let view = delegation.read_direct(&started.run_id)?;
            if view.status.is_terminal() {
                break view;
            }
        };
        guard.id = "";
        let success = completed.status == AgentRunStatus::Completed;
        (callbacks.on_tool_event)(&ToolEvent::Finished {
            sequence: 0,
            tool: "call_agent".into(),
            success,
            summary: format!("{}: {:?}", command.agent, completed.status),
            details: Some(
                serde_json::to_string(&completed).map_err(|_| "Invalid agent run result")?,
            ),
        });
        if let Some(error) = cancellation {
            return Err(error);
        }
        if !success {
            return Err(completed
                .error
                .unwrap_or_else(|| format!("{}: {:?}", command.agent, completed.status)));
        }
        let report = completed.result.ok_or("Agent completed without a report")?;
        let answer = render_report(&command.agent, &report);
        (callbacks.on_delta)(&answer);
        Ok(answer)
    }

    fn check_direct_cancelled(&self, context: &dyn RuntimeContext) -> Result<(), String> {
        if let Some(turn) = &self.turn_control {
            turn.check()?;
        }
        self.execution_scope.check_open()?;
        context.check_cancelled()
    }
}

fn render_report(agent: &str, report: &crate::agent_runs::AgentResult) -> String {
    let mut answer = format!("{} · {:?}\n\n{}", agent, report.outcome, report.summary);
    for (heading, values) in [
        ("Work done", serde_json::to_value(&report.work_done)),
        ("Findings", serde_json::to_value(&report.findings)),
        ("Artifacts", serde_json::to_value(&report.artifacts)),
        ("Verification", serde_json::to_value(&report.verification)),
        ("Remaining", serde_json::to_value(&report.remaining)),
    ] {
        if let Some(values) = values
            .ok()
            .and_then(|value| value.as_array().cloned())
            .filter(|values| !values.is_empty())
        {
            answer.push_str(&format!("\n\n{heading}:\n"));
            for value in values {
                answer.push_str("- ");
                answer.push_str(value.as_str().unwrap_or(&value.to_string()));
                answer.push('\n');
            }
        }
    }
    answer
}

struct DirectRunGuard<'a> {
    delegation: &'a crate::delegation::Delegation,
    id: &'a str,
}
impl Drop for DirectRunGuard<'_> {
    fn drop(&mut self) {
        if !self.id.is_empty() {
            let _ = self.delegation.cancel_direct(self.id);
        }
    }
}
