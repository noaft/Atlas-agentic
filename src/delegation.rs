//! Runtime-owned delegation. Definitions describe agents; callers grant tools per run.
use crate::{
    agent_catalog::{AgentCatalog, AgentDefinition},
    agent_runs::{
        AgentResult, AgentRunManager, AgentRunRequest, ContextMode, ExecutionMode, RunControl,
    },
    agents::LocalAgentDefinition,
    conversations::{ConversationMessage, MessageRole},
    permissions::{
        AuthorizationDecision, AuthorizationRequest, PermissionMode, ToolAuthorizer, ToolEvent,
        ToolRisk,
    },
    runtime::{
        context::{ContextPreparationRequest, PreparedContext, RuntimeContext},
        RuntimeAskCallbacks,
    },
    settings::{AtlasSettings, EffectiveSettings, SettingsStore},
    tools::{Tool, ToolDefinition, ToolRegistry, ToolResult},
    Runtime,
};
use serde::Deserialize;
use std::{
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

struct WakeupChildGuard {
    manager: AgentRunManager,
    owner: String,
    id: String,
}
impl Drop for WakeupChildGuard {
    fn drop(&mut self) {
        if self
            .manager
            .read(&self.owner, &self.id)
            .is_ok_and(|r| !r.status.is_terminal())
        {
            let _ = self.manager.cancel(&self.owner, &self.id);
        }
    }
}
pub type WorkerFactory = Arc<
    dyn Fn(
            &Path,
            LocalAgentDefinition,
            &EffectiveSettings,
            &SettingsStore,
        ) -> Result<Runtime, String>
        + Send
        + Sync,
>;
#[derive(Clone)]
pub struct Delegation {
    workspace: PathBuf,
    settings: EffectiveSettings,
    profiles: AtlasSettings,
    store: SettingsStore,
    catalog: AgentCatalog,
    manager: AgentRunManager,
    owner: String,
    depth: usize,
    persistent: bool,
    factory: WorkerFactory,
    execution_scope: Option<crate::executions::ExecutionScope>,
    #[cfg(any(unix, windows))]
    process_scope: Option<crate::tools::ProcessScope>,
}
impl Delegation {
    pub fn new(
        workspace: &Path,
        settings: EffectiveSettings,
        store: SettingsStore,
    ) -> Result<Self, String> {
        let catalog = AgentCatalog::for_settings(&store, workspace);
        Ok(Self {
            workspace: workspace.to_path_buf(),
            settings,
            profiles: store.load()?,
            store,
            catalog,
            manager: AgentRunManager::default(),
            owner: uuid::Uuid::new_v4().to_string(),
            depth: 0,
            persistent: false,
            execution_scope: None,
            factory: Arc::new(crate::app::build_runtime_from_definition),
            #[cfg(any(unix, windows))]
            process_scope: None,
        })
    }
    pub fn with_manager(mut self, manager: AgentRunManager, owner: String) -> Self {
        self.manager = manager;
        self.owner = owner;
        self
    }
    pub fn with_factory(mut self, factory: WorkerFactory) -> Self {
        self.factory = factory;
        self
    }
    #[cfg(any(unix, windows))]
    pub fn set_process_scope(&mut self, scope: crate::tools::ProcessScope) {
        self.owner = scope.id().into();
        self.process_scope = Some(scope);
        self.persistent = true;
    }

    #[cfg(any(unix, windows))]
    pub(crate) fn set_request_process_scope(&mut self, scope: crate::tools::ProcessScope) {
        self.owner = scope.id().into();
        self.process_scope = Some(scope);
        self.persistent = false;
    }
    pub(crate) fn close_owner(&self) {
        self.manager.close_owner(&self.owner);
    }
    pub(crate) fn set_execution_scope(&mut self, scope: crate::executions::ExecutionScope) {
        self.execution_scope = Some(scope);
    }
    pub(crate) fn service_wakeup_approvals(
        &self,
        authorizer: &dyn ToolAuthorizer,
        events: &mut dyn FnMut(&ToolEvent),
        is_child: bool,
    ) -> Result<(), String> {
        if is_child || authorizer.approvals_in_run_view() {
            return Ok(());
        }
        for run in self.manager.list(&self.owner)? {
            if let Some(request) = run.pending_approval {
                events(&ToolEvent::ApprovalRequested {
                    sequence: u32::MAX,
                    request: request.clone(),
                });
                let decision = authorizer.authorize(&request)?;
                self.manager.resolve_approval(
                    &self.owner,
                    &run.run_id,
                    &request.id,
                    decision == AuthorizationDecision::Allow,
                )?;
            }
        }
        Ok(())
    }
    pub(crate) fn prepare_wakeup(
        &self,
        tool: &str,
        args: &serde_json::Value,
        context: CallContext<'_>,
    ) -> Result<crate::executions::worker::Work, String> {
        if tool == "call_agent" {
            let result = self.call(args, context)?;
            let value: serde_json::Value =
                serde_json::from_str(&result).map_err(|_| "Invalid child run result")?;
            let id = value
                .get("run_id")
                .and_then(|v| v.as_str())
                .ok_or("Child run did not return its ID")?
                .to_string();
            let manager = self.manager.clone();
            let owner = self.owner.clone();
            let guard = WakeupChildGuard {
                manager: manager.clone(),
                owner: owner.clone(),
                id: id.clone(),
            };
            return Ok(Box::new(move |reporter| {
                let _guard = guard;
                let mut approval = None;
                let mut cancelled = false;
                loop {
                    if reporter.cancelled() && !cancelled {
                        manager.cancel(&owner, &id)?;
                        cancelled = true;
                    }
                    let view = manager.wait(&owner, &id, Duration::from_millis(100))?;
                    reporter.metadata(serde_json::json!({"run_id":id,"status":view.status,"tool_calls":view.tool_calls}));
                    let next = view.pending_approval.as_ref().map(|a| a.id.clone());
                    if next.is_some() && next != approval {
                        reporter.input_required();
                    }
                    approval = next;
                    if view.status.is_terminal() {
                        if matches!(view.status, crate::agent_runs::AgentRunStatus::Cancelled) {
                            reporter.ack_cancel();
                        }
                        if matches!(
                            view.status,
                            crate::agent_runs::AgentRunStatus::Failed
                                | crate::agent_runs::AgentRunStatus::TimedOut
                        ) {
                            return Err(view.error.unwrap_or_else(|| "Child agent failed".into()));
                        }
                        return Ok(ToolResult::text(render_run(&view, 0, 16384)?));
                    }
                }
            }));
        }
        let manager = self.manager.clone();
        let owner = self.owner.clone();
        let tool = tool.to_string();
        let args = args.clone();
        Ok(Box::new(move |_reporter| {
            let result = match tool.as_str() {
                "list_agent_runs" => {
                    parse_empty(&args)?;
                    render_list(&manager.list(&owner)?)
                }
                "read_agent_run" => {
                    let a: ReadArgs = parse(&args)?;
                    render_run(
                        &manager.read(&owner, &a.run_id)?,
                        a.output_offset.unwrap_or(0),
                        a.output_limit.unwrap_or(16384),
                    )
                }
                "cancel_agent_run" => {
                    let a: RunArgs = parse(&args)?;
                    render_run(&manager.cancel(&owner, &a.run_id)?, 0, 16384)
                }
                "wait_agent_run" => {
                    let a: WaitArgs = parse(&args)?;
                    render_run(
                        &manager.wait(
                            &owner,
                            &a.run_id,
                            Duration::from_millis(a.wait_ms.unwrap_or(1000)),
                        )?,
                        0,
                        16384,
                    )
                }
                _ => Err("Unknown agent run tool".into()),
            }?;
            Ok(ToolResult::text(result))
        }))
    }
    pub(crate) fn execute(
        &self,
        tool: &str,
        args: &serde_json::Value,
        context: CallContext<'_>,
    ) -> Result<String, String> {
        match tool {
            "call_agent" => self.call(args, context),
            "list_agent_runs" => {
                parse_empty(args)?;
                render_list(&self.manager.list(&self.owner)?)
            }
            "read_agent_run" => {
                let args: ReadArgs = parse(args)?;
                render_run(
                    &self.manager.read(&self.owner, &args.run_id)?,
                    args.output_offset.unwrap_or(0),
                    args.output_limit.unwrap_or(16384),
                )
            }
            "cancel_agent_run" => {
                let args: RunArgs = parse(args)?;
                render_run(&self.manager.cancel(&self.owner, &args.run_id)?, 0, 16384)
            }
            "wait_agent_run" => {
                let args: WaitArgs = parse(args)?;
                let wait = args.wait_ms.unwrap_or(1000);
                if wait > 10000 {
                    return Err("wait_ms must be 0–10000".into());
                }
                self.wait(&args.run_id, Duration::from_millis(wait), context)
            }
            _ => Err("Unknown agent run tool".into()),
        }
    }
    fn call(&self, args: &serde_json::Value, context: CallContext<'_>) -> Result<String, String> {
        let request: AgentRunRequest = parse(args)?;
        request.validate()?;
        if self.depth >= 4 {
            return Err("Agent delegation depth limit of 4 reached".into());
        }
        for tool in &request.tools {
            if !context
                .available_tools
                .iter()
                .any(|candidate| candidate.name == *tool)
            {
                return Err(format!("Cannot grant unavailable parent tool: {tool}"));
            }
        }
        let definition = self.catalog.load(&request.agent)?;
        let timeout = Duration::from_secs(definition.timeout_seconds);
        let settings = self.worker_settings(&definition)?;
        let mut snapshot = FrozenContext::empty(settings.request_output_tokens);
        if request.context_mode == ContextMode::Fork {
            snapshot.history = context.prepared.history.clone();
            snapshot.memory_blocks = context.prepared.memory_blocks.clone();
            snapshot.activity_context = context.prepared.activity_context.clone();
            snapshot.history.push(ConversationMessage {
                role: MessageRole::User,
                content: context.parent_input.to_string(),
                created_at: chrono::Utc::now(),
            });
        }
        let mut child = self.clone();
        child.settings = settings.clone();
        child.depth += 1;
        let mode = context.permission_mode;
        let task = request.task.clone();
        let allowed = request.tools.clone();
        let execution = request.execution_mode;
        let view = self.manager.start_with_parent(
            &self.owner,
            request,
            timeout,
            context.control.map(RunControl::run_id),
            move |control| {
                control.check()?;
                let local = local_definition(&definition, &settings);
                let mut runtime =
                    (child.factory)(&child.workspace, local, &settings, &child.store)?;
                if let Some(scope) = &child.execution_scope {
                    let execution_scope = scope.child(control.run_id());
                    control.attach_execution_scope(execution_scope.clone())?;
                    runtime = runtime.with_execution_scope(execution_scope);
                }
                #[cfg(any(unix, windows))]
                if let Some(scope) = &child.process_scope {
                    runtime = if child.persistent {
                        runtime.with_process_scope(&child.workspace, scope.clone())
                    } else {
                        runtime.with_request_process_scope(&child.workspace, scope.clone(), true)
                    };
                }
                // Register then restrict: scope registration must never restore an ungranted tool.
                let skills =
                    crate::skills::SkillStore::for_settings(&child.store, child.workspace.clone())?;
                runtime = runtime.select_skills(&skills, &definition.skills)?;
                runtime = runtime
                    .with_delegation(child.clone())
                    .restrict_tools(&allowed);
                runtime.run_control = Some(control.clone());
                control.set_model(&settings.model);
                let authorizer = RunAuthorizer(control.clone());
                let mut last_progress: Option<String> = None;
                let mut events = |event: &ToolEvent| match event {
                    ToolEvent::Progress { message } => {
                        last_progress = Some(message.clone());
                        control.update(last_progress.as_deref(), None);
                    }
                    ToolEvent::Usage { usage } => control.record_usage(usage.as_ref()),
                    ToolEvent::Started { tool, .. } => {
                        control.increment_tool_calls();
                        control.update(last_progress.as_deref(), Some(tool));
                    }
                    ToolEvent::Finished {
                        tool,
                        success,
                        summary,
                        ..
                    } => {
                        control.record_tool(tool, Some(*success), summary);
                        control.update(last_progress.as_deref(), None);
                    }
                    _ => {}
                };
                let output = runtime.ask_stream_with_managed_context(
                    &task,
                    &mut snapshot,
                    mode,
                    RuntimeAskCallbacks {
                        authorizer: &authorizer,
                        on_delta: &mut |_| {},
                        on_tool_event: &mut events,
                    },
                );
                control.check()?;
                // Provider/transport errors are already sanitized at their boundary; no response body is retained.
                AgentResult::parse(&output?)
            },
        )?;
        if execution == ExecutionMode::Wait {
            self.wait(&view.run_id, timeout, context)
        } else {
            render_run(&view, 0, 16384)
        }
    }
    fn wait(
        &self,
        id: &str,
        duration: Duration,
        context: CallContext<'_>,
    ) -> Result<String, String> {
        if duration.is_zero() {
            return render_run(&self.manager.read(&self.owner, id)?, 0, 16384);
        }
        let deadline = std::time::Instant::now() + duration;
        let mut notified = std::collections::HashSet::new();
        loop {
            if let Some(control) = context.control {
                control.check()?;
            }
            let view = self.manager.wait(
                &self.owner,
                id,
                deadline
                    .saturating_duration_since(std::time::Instant::now())
                    .min(Duration::from_millis(100)),
            )?;
            // Background/native approvals remain attached to their real owning run.
            // CLI foreground waits service pending approvals across descendants without duplicating them.
            for pending in self.manager.list(&self.owner)? {
                if let Some(request) = &pending.pending_approval {
                    if context.control.is_none() && !context.authorizer.approvals_in_run_view() {
                        (context.on_tool_event)(&ToolEvent::ApprovalRequested {
                            sequence: context.sequence,
                            request: request.clone(),
                        });
                        let decision = context.authorizer.authorize(request)?;
                        let _ = self.manager.resolve_approval(
                            &self.owner,
                            &pending.run_id,
                            &request.id,
                            decision == AuthorizationDecision::Allow,
                        );
                    } else if context.control.is_none() && notified.insert(request.id.clone()) {
                        (context.on_tool_event)(&ToolEvent::Progress {
                            message: format!(
                                "{} needs approval for {}. Review the agent card in this conversation.",
                                pending.agent, request.tool
                            ),
                        });
                    }
                }
            }
            if view.status.is_terminal() || std::time::Instant::now() >= deadline {
                return render_run(&self.manager.read(&self.owner, id)?, 0, 16384);
            }
        }
    }
    fn worker_settings(&self, definition: &AgentDefinition) -> Result<EffectiveSettings, String> {
        match (&definition.provider, &definition.model) {
            (None, None) => Ok(self.settings.clone()),
            (Some(provider), Some(model)) => {
                // Explicit agent overrides select that provider profile, never another provider's credentials.
                let mut profiles = self.profiles.clone();
                profiles.selected_provider = Some(crate::settings::ProviderId::parse(provider)?);
                let mut settings = profiles.resolve_for(provider, model, |name| {
                    if name == "ATLAS_PROVIDER" {
                        None
                    } else {
                        std::env::var_os(name)
                    }
                })?;
                settings.model = model.clone();
                settings.model_from_agent = false;
                Ok(settings)
            }
            _ => Err("Agent provider and model must be supplied together".into()),
        }
    }
}
fn local_definition(
    definition: &AgentDefinition,
    settings: &EffectiveSettings,
) -> LocalAgentDefinition {
    LocalAgentDefinition {
        name: definition.name.clone(),
        display_name: definition.display_name.clone(),
        model_provider: settings.provider.as_str().into(),
        model: settings.model.clone(),
        temperature: None,
        top_p: None,
        max_tool_calls: Some(definition.max_tool_calls),
        max_protocol_repairs: Some(3),
        system_prompt: format!(
            "{}\n\n{}",
            definition.system_prompt,
            include_str!("../config/agent-result-prompt.md")
        ),
        conversation_title_prompt: String::new(),
        conversation_summary_prompt: None,
        memory: Default::default(),
    }
}
pub(crate) struct CallContext<'a> {
    pub prepared: &'a PreparedContext,
    pub parent_input: &'a str,
    pub available_tools: &'a [ToolDefinition],
    pub permission_mode: PermissionMode,
    pub authorizer: &'a dyn ToolAuthorizer,
    pub on_tool_event: &'a mut dyn FnMut(&ToolEvent),
    pub sequence: u32,
    pub control: Option<&'a RunControl>,
}
struct RunAuthorizer(RunControl);
impl ToolAuthorizer for RunAuthorizer {
    fn approvals_in_run_view(&self) -> bool {
        true
    }
    fn authorize(&self, request: &AuthorizationRequest) -> Result<AuthorizationDecision, String> {
        self.0.authorize(request)
    }
}
struct FrozenContext {
    history: Vec<ConversationMessage>,
    memory_blocks: Vec<crate::conversations::MemoryBlock>,
    activity_context: String,
    output_limit: u64,
}
impl FrozenContext {
    fn empty(output_limit: u64) -> Self {
        Self {
            history: vec![],
            memory_blocks: vec![],
            activity_context: String::new(),
            output_limit,
        }
    }
}
impl RuntimeContext for FrozenContext {
    fn prepare(
        &mut self,
        agent: &dyn crate::agents::Agent,
        request: ContextPreparationRequest<'_>,
        _: &mut dyn FnMut(&ToolEvent),
    ) -> Result<PreparedContext, String> {
        let mut max_output_tokens = None;
        if let Some(limits) = agent.model_limits()? {
            limits.validate()?;
            let reserve = limits.max_output_tokens.min(self.output_limit);
            let input = crate::agents::AgentRequest {
                input: request.input,
                history: &self.history,
                memory_blocks: &self.memory_blocks,
                tools: request.tools,
                activity_context: Some(&self.activity_context),
                images: request.images,
                max_output_tokens: Some(reserve),
            };
            if let Some(estimate) = agent.estimate_request(&input)? {
                if estimate.tokens.saturating_add(reserve) >= limits.context_window {
                    return Err("Fork/task context exceeds child model capacity; choose task_only with a smaller task or a larger model".into());
                }
            }
            max_output_tokens = Some(reserve);
        }
        Ok(PreparedContext {
            history: self.history.clone(),
            memory_blocks: self.memory_blocks.clone(),
            activity_context: self.activity_context.clone(),
            max_output_tokens,
        })
    }
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RunArgs {
    run_id: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ReadArgs {
    run_id: String,
    output_offset: Option<usize>,
    output_limit: Option<usize>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WaitArgs {
    run_id: String,
    wait_ms: Option<u64>,
}
fn parse<T: serde::de::DeserializeOwned>(args: &serde_json::Value) -> Result<T, String> {
    serde_json::from_value(args.clone()).map_err(|_| "Invalid agent tool arguments".into())
}
fn parse_empty(args: &serde_json::Value) -> Result<(), String> {
    if args.as_object().is_some_and(|o| o.is_empty()) {
        Ok(())
    } else {
        Err("Expected empty object arguments".into())
    }
}
fn json<T: serde::Serialize>(value: &T) -> Result<String, String> {
    serde_json::to_string(value).map_err(|_| "Cannot serialize agent run".into())
}
fn clip(text: &str, limit: usize) -> String {
    text.chars().take(limit).collect()
}
fn metadata(view: &crate::agent_runs::AgentRunView) -> serde_json::Value {
    serde_json::json!({"run_id":view.run_id,"agent":clip(&view.agent,128),"status":view.status,"context_mode":view.context_mode,"execution_mode":view.execution_mode,"duration_ms":view.duration_ms,"tool_calls":view.tool_calls,"model":view.model,"progress":view.progress.as_deref().map(|s|clip(s,256)),"current_tool":view.current_tool,"needs_approval":view.pending_approval.is_some(),"has_result":view.result.is_some()})
}
fn render_list(views: &[crate::agent_runs::AgentRunView]) -> Result<String, String> {
    let mut data = vec![];
    for view in views {
        data.push(metadata(view));
        if json(&serde_json::json!({"data":data,"truncated":false}))?.len() > 64000 {
            data.pop();
            break;
        }
    }
    let truncated = data.len() < views.len();
    json(&serde_json::json!({"data":data,"truncated":truncated,"total":views.len()}))
}
fn render_run(
    view: &crate::agent_runs::AgentRunView,
    offset: usize,
    limit: usize,
) -> Result<String, String> {
    if !(1..=16384).contains(&limit) {
        return Err("output_limit must be 1–16384 bytes".into());
    }
    let report = match &view.result {
        Some(result) => json(result)?,
        None => String::new(),
    };
    if offset > report.len() || !report.is_char_boundary(offset) {
        return Err("output_offset must be a valid result UTF-8 byte boundary".into());
    }
    let mut end = offset.saturating_add(limit).min(report.len());
    while !report.is_char_boundary(end) {
        end -= 1;
    }
    if end == offset && end < report.len() {
        return Err("output_limit is too small for the next UTF-8 character".into());
    }
    let mut value = metadata(view);
    value["task"] = clip(&view.task, 1024).into();
    value["error"] = view.error.as_deref().map(|s| clip(s, 1024)).into();
    value["result_output"] = report[offset..end].into();
    value["output_offset"] = offset.into();
    value["output_bytes"] = report.len().into();
    value["next_cursor"] = if end < report.len() {
        end.into()
    } else {
        serde_json::Value::Null
    };
    value["result"] = if offset == 0 && end == report.len() {
        serde_json::to_value(&view.result).map_err(|_| "Cannot serialize report")?
    } else {
        serde_json::Value::Null
    };
    let output = json(&value)?;
    if output.len() > 65536 {
        return Err(
            "Agent result page exceeds output bound; retry with a smaller output_limit".into(),
        );
    }
    Ok(output)
}
pub(crate) fn is_run_tool(name: &str) -> bool {
    matches!(
        name,
        "call_agent" | "list_agent_runs" | "read_agent_run" | "wait_agent_run" | "cancel_agent_run"
    )
}
pub(crate) fn register_tools(registry: &mut ToolRegistry) {
    for name in [
        "call_agent",
        "list_agent_runs",
        "read_agent_run",
        "wait_agent_run",
        "cancel_agent_run",
    ] {
        registry.register(Box::new(DelegationTool::new(name)));
    }
}
struct DelegationTool {
    name: &'static str,
}
impl DelegationTool {
    fn new(name: &'static str) -> Self {
        Self { name }
    }
}
impl Tool for DelegationTool {
    fn name(&self) -> &str {
        self.name
    }
    fn definition(&self) -> ToolDefinition {
        let (description,schema)=match self.name {
            "call_agent"=>("Start a reusable agent with ONLY caller-selected tools. Discover IDs using list_agents. fork snapshots current context; task_only receives only its task. wait waits, parallel returns run_id while the worker continues. Use read/wait for the structured final report. Tools must be a subset of your own tools.",serde_json::json!({"type":"object","additionalProperties":false,"required":["agent","task","tools","context_mode","execution_mode"],"properties":{"agent":{"type":"string"},"task":{"type":"string"},"tools":{"type":"array","items":{"type":"string"},"maxItems":64},"context_mode":{"enum":["fork","task_only"]},"execution_mode":{"enum":["wait","parallel"]}}})),
            "list_agent_runs"=>("List bounded agent runs owned by this chat.",serde_json::json!({"type":"object","properties":{},"additionalProperties":false})),
            name=> (if name=="cancel_agent_run" {"Request cancellation of an owned agent run; cancelling is not stopped."} else {"Read an owned run status and structured result. Long reports use result_output pages: pass next_cursor as output_offset to read_agent_run. output_limit is 1–16384 bytes. wait_ms waits up to 10000ms with wait_agent_run."},serde_json::json!({"type":"object","required":["run_id"],"additionalProperties":false,"properties":{"run_id":{"type":"string"}}})),
        };
        let mut schema = schema;
        if self.name == "read_agent_run" {
            schema["properties"]["output_offset"] =
                serde_json::json!({"type":"integer","minimum":0});
            schema["properties"]["output_limit"] =
                serde_json::json!({"type":"integer","minimum":1,"maximum":16384});
        }
        if self.name == "wait_agent_run" {
            schema["properties"]["wait_ms"] =
                serde_json::json!({"type":"integer","minimum":0,"maximum":10000});
        }
        ToolDefinition {
            name: self.name.into(),
            description: description.into(),
            input_schema: schema,
        }
    }
    fn risk(&self, args: &serde_json::Value) -> Result<ToolRisk, String> {
        match self.name {
            "call_agent" => parse::<AgentRunRequest>(args)?.validate()?,
            "list_agent_runs" => parse_empty(args)?,
            "read_agent_run" => {
                let a: ReadArgs = parse(args)?;
                if a.output_limit.is_some_and(|n| !(1..=16384).contains(&n)) {
                    return Err("output_limit must be 1–16384".into());
                }
            }
            "wait_agent_run" => {
                let a: WaitArgs = parse(args)?;
                if a.wait_ms.is_some_and(|n| n > 10000) {
                    return Err("wait_ms must be 0–10000".into());
                }
            }
            _ => {
                let a: RunArgs = parse(args)?;
                if a.run_id.is_empty() {
                    return Err("run_id is required".into());
                }
            }
        }
        Ok(if self.name == "cancel_agent_run" {
            ToolRisk::Command
        } else {
            ToolRisk::ReadOnly
        })
    }
    fn execute(&self, _: &serde_json::Value) -> Result<ToolResult, String> {
        Err("Agent run tools require runtime context".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::{PwdTool, Tool};
    #[test]
    fn call_validation_rejects_unknown_modes_and_empty_tools_are_explicit() {
        let call = serde_json::json!({"agent":"personal:reviewer","task":"Review", "tools":[], "context_mode":"task_only","execution_mode":"parallel"});
        assert!(DelegationTool::new("call_agent").risk(&call).is_ok());
        let mut invalid = call.clone();
        invalid["context_mode"] = "live".into();
        assert!(DelegationTool::new("call_agent").risk(&invalid).is_err());
    }
    #[test]
    fn model_run_inspection_pages_reports_and_bounds_list_metadata() {
        let manager = AgentRunManager::default();
        let request:AgentRunRequest=serde_json::from_value(serde_json::json!({"agent":"personal:test","task":"x".repeat(65536),"tools":[],"context_mode":"task_only","execution_mode":"wait"})).unwrap();
        let initial=manager.start("owner",request,Duration::from_secs(2),|_|AgentResult::parse(&serde_json::json!({"outcome":"done","summary":"Finished","work_done":["🦀".repeat(12000)],"findings":[],"artifacts":[],"verification":[],"remaining":[]}).to_string())).unwrap();
        let view = manager
            .wait("owner", &initial.run_id, Duration::from_secs(1))
            .unwrap();
        let listing = render_list(std::slice::from_ref(&view)).unwrap();
        assert!(listing.len() <= 65536);
        assert!(!listing.contains("🦀"));
        let mut offset = 0;
        let mut reconstructed = String::new();
        loop {
            let page = render_run(&view, offset, 1024).unwrap();
            assert!(page.len() <= 65536);
            let value: serde_json::Value = serde_json::from_str(&page).unwrap();
            reconstructed.push_str(value["result_output"].as_str().unwrap());
            if let Some(next) = value["next_cursor"].as_u64() {
                offset = next as usize
            } else {
                break;
            }
        }
        assert_eq!(
            AgentResult::parse(&reconstructed).unwrap().work_done[0],
            "🦀".repeat(12000)
        );
    }
    #[test]
    fn tool_restriction_removes_executable_authority_not_only_metadata() {
        let folder = tempfile::tempdir().unwrap();
        let mut registry =
            crate::tools::ToolRegistry::new(vec![Box::new(PwdTool::new(folder.path()))]);
        registry.retain_names(&[]);
        assert!(registry.definitions().is_empty());
        assert!(registry
            .authorization("pwd", &serde_json::json!({}))
            .is_err());
    }
}

#[cfg(test)]
mod integration_tests {
    use super::*;
    use crate::{
        agents::{AgentRegistry, LocalAgent},
        models::{ModelProvider, ModelRequest},
        permissions::AllowAllAuthorizer,
        tools::PwdTool,
    };
    use std::sync::Mutex;
    struct ParentModel(Mutex<Vec<String>>);
    impl ModelProvider for ParentModel {
        fn complete(&self, _: &ModelRequest) -> Result<String, String> {
            Ok(self.0.lock().unwrap().remove(0))
        }
    }
    struct ChildModel(Arc<Mutex<Vec<serde_json::Value>>>);
    impl ModelProvider for ChildModel {
        fn complete(&self, r: &ModelRequest) -> Result<String, String> {
            self.0.lock().unwrap().push(serde_json::json!({"history":r.history.iter().map(|m|&m.content).collect::<Vec<_>>(),"memory":r.memory_blocks.len(),"tools":r.tools.iter().map(|t|&t.name).collect::<Vec<_>>(),"input":r.user_prompt}));
            Ok(final_reply(&serde_json::json!({"outcome":"done","summary":"Checked","work_done":["Read task"],"findings":[],"artifacts":[],"verification":[],"remaining":[]}).to_string()))
        }
    }
    fn final_reply(content: &str) -> String {
        format!(
            "{{\"type\":\"final\",\"content\":{}}}",
            serde_json::to_string(content).unwrap()
        )
    }
    #[test]
    fn cancellation_during_context_preparation_does_not_start_another_model_call() {
        use std::sync::{
            atomic::{AtomicUsize, Ordering},
            mpsc,
        };
        struct GateModel {
            entered: mpsc::Sender<()>,
            release: Mutex<mpsc::Receiver<()>>,
            calls: Arc<AtomicUsize>,
        }
        impl ModelProvider for GateModel {
            fn model_limits(&self) -> Result<Option<crate::models::ModelLimits>, String> {
                self.entered.send(()).unwrap();
                self.release.lock().unwrap().recv().unwrap();
                Ok(None)
            }
            fn complete(&self, _: &ModelRequest) -> Result<String, String> {
                self.calls.fetch_add(1, Ordering::SeqCst);
                Ok(final_reply("{}"))
            }
        }
        let manager = AgentRunManager::default();
        let (entered, ready) = mpsc::channel();
        let (release, gate) = mpsc::channel();
        let calls = Arc::new(AtomicUsize::new(0));
        let recorded = calls.clone();
        let request:AgentRunRequest=serde_json::from_value(serde_json::json!({"agent":"helper","task":"Work","tools":[],"context_mode":"task_only","execution_mode":"parallel"})).unwrap();
        let view = manager
            .start("owner", request, Duration::from_secs(5), move |control| {
                let local =
                    LocalAgentDefinition::from_toml(include_str!("../agents/atlas.toml")).unwrap();
                let mut runtime =
                    Runtime::new(AgentRegistry::new(vec![Box::new(LocalAgent::new(
                        local,
                        Box::new(GateModel {
                            entered,
                            release: Mutex::new(gate),
                            calls: recorded,
                        }),
                    ))]));
                runtime.run_control = Some(control);
                let mut context = FrozenContext::empty(4096);
                let output = runtime.ask_stream_with_managed_context(
                    "Work",
                    &mut context,
                    PermissionMode::FullAccess,
                    RuntimeAskCallbacks {
                        authorizer: &AllowAllAuthorizer,
                        on_delta: &mut |_| {},
                        on_tool_event: &mut |_| {},
                    },
                )?;
                AgentResult::parse(&output)
            })
            .unwrap();
        ready.recv_timeout(Duration::from_secs(1)).unwrap();
        manager.cancel("owner", &view.run_id).unwrap();
        release.send(()).unwrap();
        assert!(manager
            .wait("owner", &view.run_id, Duration::from_secs(1))
            .unwrap()
            .status
            .is_terminal());
        assert_eq!(calls.load(Ordering::SeqCst), 0);
    }
    #[test]
    fn child_runtime_requires_its_own_scoped_approval_before_writing() {
        let folder = tempfile::tempdir().unwrap();
        let workspace = folder.path().to_path_buf();
        let manager = AgentRunManager::default();
        let request:AgentRunRequest=serde_json::from_value(serde_json::json!({"agent":"writer","task":"Write","tools":["create_file"],"context_mode":"task_only","execution_mode":"parallel"})).unwrap();
        let call = format!(
            "{{\"type\":\"tool_call\",\"tool\":\"create_file\",\"args\":{}}}",
            serde_json::json!({"path":"approved.txt","content":"written"})
        );
        let report=serde_json::json!({"outcome":"done","summary":"Wrote file","work_done":["Created approved.txt"],"findings":[],"artifacts":["approved.txt"],"verification":[],"remaining":[]}).to_string();
        let view = manager
            .start("owner", request, Duration::from_secs(5), move |control| {
                let local =
                    LocalAgentDefinition::from_toml(include_str!("../agents/atlas.toml")).unwrap();
                let mut runtime =
                    Runtime::new(AgentRegistry::new(vec![Box::new(LocalAgent::new(
                        local,
                        Box::new(ParentModel(Mutex::new(vec![call, final_reply(&report)]))),
                    ))]))
                    .with_tools(ToolRegistry::new(vec![Box::new(
                        crate::tools::CreateFileTool::new(&workspace),
                    )]));
                runtime.run_control = Some(control.clone());
                let authorizer = RunAuthorizer(control);
                let mut snapshot = FrozenContext::empty(4096);
                let answer = runtime.ask_stream_with_managed_context(
                    "Write",
                    &mut snapshot,
                    PermissionMode::Ask,
                    RuntimeAskCallbacks {
                        authorizer: &authorizer,
                        on_delta: &mut |_| {},
                        on_tool_event: &mut |_| {},
                    },
                )?;
                AgentResult::parse(&answer)
            })
            .unwrap();
        let until = std::time::Instant::now() + Duration::from_secs(2);
        let pending = loop {
            let view = manager.read("owner", &view.run_id).unwrap();
            if view.pending_approval.is_some() {
                break view;
            }
            assert!(std::time::Instant::now() < until);
            std::thread::sleep(Duration::from_millis(5));
        };
        assert!(!folder.path().join("approved.txt").exists());
        let approval = pending.pending_approval.unwrap();
        assert!(manager
            .resolve_approval("foreign", &view.run_id, &approval.id, true)
            .is_err());
        manager
            .resolve_approval("owner", &view.run_id, &approval.id, true)
            .unwrap();
        assert_eq!(
            manager
                .wait("owner", &view.run_id, Duration::from_secs(1))
                .unwrap()
                .status,
            crate::agent_runs::AgentRunStatus::Completed
        );
        assert_eq!(
            std::fs::read_to_string(folder.path().join("approved.txt")).unwrap(),
            "written"
        );
    }
    #[test]
    fn grandchildren_inherit_their_immediate_parents_overridden_model() {
        use std::cell::Cell;
        struct NestedModel {
            middle: bool,
            calls: Cell<usize>,
        }
        impl ModelProvider for NestedModel {
            fn complete(&self, _: &ModelRequest) -> Result<String, String> {
                let call = self.calls.get();
                self.calls.set(call + 1);
                if self.middle && call == 0 {
                    Ok(format!(
                        "{{\"type\":\"tool_call\",\"tool\":\"call_agent\",\"args\":{}}}",
                        serde_json::json!({"agent":"personal:leaf","task":"Work","tools":[],"context_mode":"task_only","execution_mode":"wait"})
                    ))
                } else {
                    Ok(final_reply(&serde_json::json!({"outcome":"done","summary":"Done","work_done":[],"findings":[],"artifacts":[],"verification":[],"remaining":[]}).to_string()))
                }
            }
        }
        let folder = tempfile::tempdir().unwrap();
        let store = SettingsStore::at(folder.path().join("settings.json"));
        let catalog = AgentCatalog::for_settings(&store, folder.path());
        let mut middle = definition();
        middle.name = "middle".into();
        middle.provider = Some("deepseek".into());
        middle.model = Some("child-model".into());
        catalog.create("personal", middle).unwrap();
        let mut leaf = definition();
        leaf.name = "leaf".into();
        catalog.create("personal", leaf).unwrap();
        let models = Arc::new(Mutex::new(vec![]));
        let captured = models.clone();
        let settings = EffectiveSettings {
            model: "root-model".into(),
            ..Default::default()
        };
        let delegation = Delegation::new(folder.path(), settings, store)
            .unwrap()
            .with_factory(Arc::new(move |_, definition, settings, _| {
                captured.lock().unwrap().push(settings.model.clone());
                let middle = definition.name == "middle";
                Ok(Runtime::new(AgentRegistry::new(vec![Box::new(
                    LocalAgent::new(
                        definition,
                        Box::new(NestedModel {
                            middle,
                            calls: Cell::new(0),
                        }),
                    ),
                )])))
            }));
        let local = LocalAgentDefinition::from_toml(include_str!("../agents/atlas.toml")).unwrap();
        let call = format!(
            "{{\"type\":\"tool_call\",\"tool\":\"call_agent\",\"args\":{}}}",
            serde_json::json!({"agent":"personal:middle","task":"Work","tools":["call_agent"],"context_mode":"task_only","execution_mode":"wait"})
        );
        let runtime = Runtime::new(AgentRegistry::new(vec![Box::new(LocalAgent::new(
            local,
            Box::new(ParentModel(Mutex::new(vec![call, final_reply("Done")]))),
        ))]))
        .with_delegation(delegation);
        runtime
            .ask_stream_with_history_and_authorization(
                "Work",
                &[],
                PermissionMode::FullAccess,
                &AllowAllAuthorizer,
                &mut |_| {},
                &mut |_| {},
            )
            .unwrap();
        assert_eq!(*models.lock().unwrap(), vec!["child-model", "child-model"]);
    }
    #[cfg(unix)]
    #[test]
    fn null_process_read_options_cannot_consume_the_parents_default_cursor() {
        let folder = tempfile::tempdir().unwrap();
        let processes = crate::tools::ProcessManager::new();
        let scope = processes.create_scope().unwrap();
        let mut parent_tools = ToolRegistry::empty();
        crate::tools::register_process_tools_in_scope(
            &mut parent_tools,
            folder.path(),
            scope.clone(),
        );
        for option in ["cursor", "tail_lines"] {
            let output=parent_tools.execute(&crate::agent_output::AgentOutput::ToolCall {message:None,tool:"exec_command".into(),args:serde_json::json!({"command":"printf 'parent bytes\n'","yield_time_ms":0})}).unwrap();
            let output: serde_json::Value = serde_json::from_str(&output.message).unwrap();
            let id = output["session_id"].as_str().unwrap().to_string();
            let snapshot = scope
                .read(&id, Some(crate::tools::ProcessCursor::default()), 1000)
                .unwrap();
            assert_eq!(snapshot.stdout, "parent bytes\n");
            let mut args = serde_json::json!({"session_id":id});
            args[option] = serde_json::Value::Null;
            let call =
                format!("{{\"type\":\"tool_call\",\"tool\":\"read_process\",\"args\":{args}}}");
            let report=serde_json::json!({"outcome":"done","summary":"Done","work_done":[],"findings":[],"artifacts":[],"verification":[],"remaining":[]}).to_string();
            let manager = AgentRunManager::default();
            let owned = scope.clone();
            let workspace = folder.path().to_path_buf();
            let request:AgentRunRequest=serde_json::from_value(serde_json::json!({"agent":"reader","task":"Read","tools":["read_process"],"context_mode":"task_only","execution_mode":"parallel"})).unwrap();
            let child = manager
                .start("owner", request, Duration::from_secs(5), move |control| {
                    let local =
                        LocalAgentDefinition::from_toml(include_str!("../agents/atlas.toml"))
                            .unwrap();
                    let mut runtime =
                        Runtime::new(AgentRegistry::new(vec![Box::new(LocalAgent::new(
                            local,
                            Box::new(ParentModel(Mutex::new(vec![call, final_reply(&report)]))),
                        ))]))
                        .with_process_scope(&workspace, owned)
                        .restrict_tools(&["read_process".into()]);
                    runtime.run_control = Some(control);
                    let answer = runtime.ask_stream_with_history_and_authorization(
                        "Read",
                        &[],
                        PermissionMode::FullAccess,
                        &AllowAllAuthorizer,
                        &mut |_| {},
                        &mut |_| {},
                    )?;
                    AgentResult::parse(&answer)
                })
                .unwrap();
            assert!(manager
                .wait("owner", &child.run_id, Duration::from_secs(2))
                .unwrap()
                .status
                .is_terminal());
            assert_eq!(
                scope.read(&id, None, 0).unwrap().stdout,
                "parent bytes\n",
                "{option}:null must not advance the parent's reader"
            );
        }
        scope.close(Duration::from_secs(2)).unwrap();
    }
    fn definition() -> AgentDefinition {
        serde_json::from_value(serde_json::json!({"name":"reviewer","display_name":"Reviewer","description":"Review","system_prompt":"Review task"})).unwrap()
    }
    fn setup(
        context: &str,
        execution: &str,
        tools: Vec<&str>,
    ) -> (
        tempfile::TempDir,
        Runtime,
        AgentRunManager,
        Arc<Mutex<Vec<serde_json::Value>>>,
    ) {
        let dir = tempfile::tempdir().unwrap();
        let store = SettingsStore::at(dir.path().join("settings.json"));
        AgentCatalog::for_settings(&store, dir.path())
            .create("personal", definition())
            .unwrap();
        let seen = Arc::new(Mutex::new(vec![]));
        let captured = seen.clone();
        let manager = AgentRunManager::default();
        let delegation = Delegation::new(dir.path(), EffectiveSettings::default(), store)
            .unwrap()
            .with_manager(manager.clone(), "test-chat".into())
            .with_factory(Arc::new(move |path, definition, _, _| {
                Ok(
                    Runtime::new(AgentRegistry::new(vec![Box::new(LocalAgent::new(
                        definition,
                        Box::new(ChildModel(captured.clone())),
                    ))]))
                    .with_tools(ToolRegistry::new(vec![Box::new(PwdTool::new(path))])),
                )
            }));
        let local = LocalAgentDefinition::from_toml(include_str!("../agents/atlas.toml")).unwrap();
        let call = format!(
            "{{\"type\":\"tool_call\",\"tool\":\"call_agent\",\"args\":{}}}",
            serde_json::json!({"agent":"personal:reviewer","task":"Review this","tools":tools,"context_mode":context,"execution_mode":execution})
        );
        let runtime = Runtime::new(AgentRegistry::new(vec![Box::new(LocalAgent::new(
            local,
            Box::new(ParentModel(Mutex::new(vec![
                call,
                final_reply("parent complete"),
            ]))),
        ))]))
        .with_tools(ToolRegistry::new(vec![Box::new(PwdTool::new(dir.path()))]))
        .with_delegation(delegation);
        (dir, runtime, manager, seen)
    }
    #[test]
    fn wait_task_only_runs_with_exact_granted_tools_and_structured_result() {
        let (_dir, runtime, manager, seen) = setup("task_only", "wait", vec!["pwd"]);
        let history = vec![ConversationMessage {
            role: MessageRole::User,
            content: "private parent history".into(),
            created_at: chrono::Utc::now(),
        }];
        assert_eq!(
            runtime
                .ask_stream_with_history_and_authorization(
                    "parent request",
                    &history,
                    PermissionMode::FullAccess,
                    &AllowAllAuthorizer,
                    &mut |_| {},
                    &mut |_| {}
                )
                .unwrap(),
            "parent complete"
        );
        let views = manager.list("test-chat").unwrap();
        assert_eq!(views.len(), 1);
        assert_eq!(
            views[0].status,
            crate::agent_runs::AgentRunStatus::Completed
        );
        assert_eq!(views[0].result.as_ref().unwrap().summary, "Checked");
        let seen = seen.lock().unwrap();
        assert_eq!(seen[0]["history"], serde_json::json!([]));
        assert_eq!(seen[0]["tools"], serde_json::json!(["pwd"]));
    }
    #[test]
    fn fork_parallel_survives_parent_final_and_cannot_restore_ungranted_tools() {
        let (_dir, runtime, manager, seen) = setup("fork", "parallel", vec![]);
        let history = vec![ConversationMessage {
            role: MessageRole::User,
            content: "parent context".into(),
            created_at: chrono::Utc::now(),
        }];
        runtime
            .ask_stream_with_history_and_authorization(
                "current task",
                &history,
                PermissionMode::FullAccess,
                &AllowAllAuthorizer,
                &mut |_| {},
                &mut |_| {},
            )
            .unwrap();
        let id = manager.list("test-chat").unwrap()[0].run_id.clone();
        let view = manager
            .wait("test-chat", &id, Duration::from_secs(2))
            .unwrap();
        assert_eq!(view.status, crate::agent_runs::AgentRunStatus::Completed);
        let seen = seen.lock().unwrap();
        assert!(seen[0]["history"]
            .as_array()
            .unwrap()
            .iter()
            .any(|v| v == "parent context"));
        assert_eq!(seen[0]["tools"], serde_json::json!([]));
    }
    #[test]
    fn parent_cannot_grant_a_tool_it_does_not_have() {
        let (_dir, runtime, manager, seen) = setup("task_only", "wait", vec!["exec_command"]);
        runtime
            .ask_stream_with_history_and_authorization(
                "task",
                &[],
                PermissionMode::FullAccess,
                &AllowAllAuthorizer,
                &mut |_| {},
                &mut |_| {},
            )
            .unwrap();
        assert!(manager.list("test-chat").unwrap().is_empty());
        assert!(seen.lock().unwrap().is_empty());
    }
}
