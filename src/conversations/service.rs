use std::{path::Path, time::Instant};

use crate::permissions::{
    MissingAuthorizer, PermissionMode, RuntimeStatus, ToolAuthorizer, ToolEvent,
};
use crate::{Runtime, RuntimeAskCallbacks};

use super::{
    render_activity_context, sanitize_title, select_recent_history, ActivityCollector,
    ActivityRecord, ConversationId, ConversationMetadata, ConversationStore,
};

#[derive(Debug)]
pub struct ConversationAskResult {
    pub answer: String,
    pub additional_inputs: Vec<crate::turns::TurnInput>,
    pub interrupted: bool,
    pub metadata: ConversationMetadata,
    pub activities: Vec<ActivityRecord>,
    pub activity_duration_ms: u64,
    pub usage: Option<crate::models::ModelUsage>,
    pub memory: Option<super::MemoryCheckpoint>,
    pub created: bool,
    pub persistence_warning: Option<String>,
}

impl ConversationAskResult {
    fn warn_after_commit(&mut self, error: &str) {
        if self.persistence_warning.is_none() {
            self.persistence_warning=Some(format!("Conversation saved; some session details could not be written: {error}. Do not resend this turn automatically."));
        }
        crate::diagnostics::event(
            "ERROR",
            "session.persistence_incomplete",
            serde_json::json!({"conversationId":self.metadata.id,"error":error}),
        );
    }
}

pub struct AskCallbacks<'a> {
    pub authorizer: &'a dyn ToolAuthorizer,
    pub on_delta: &'a mut dyn FnMut(&str) -> Result<(), String>,
    pub on_tool_event: &'a mut dyn FnMut(&ToolEvent),
}

pub struct ConversationService<'a> {
    runtime: &'a Runtime,
    store: &'a ConversationStore,
    history_limit: usize,
    cancelled: Option<&'a std::sync::atomic::AtomicBool>,
    session_key: Option<&'a str>,
}

impl<'a> ConversationService<'a> {
    pub fn new(runtime: &'a Runtime, store: &'a ConversationStore, history_limit: usize) -> Self {
        Self {
            runtime,
            store,
            history_limit,
            cancelled: None,
            session_key: None,
        }
    }

    pub fn with_session_key(mut self, key: &'a str) -> Self {
        self.session_key = Some(key);
        self
    }
    pub fn with_cancellation(mut self, cancelled: &'a std::sync::atomic::AtomicBool) -> Self {
        self.cancelled = Some(cancelled);
        self
    }
    pub fn compact(
        &self,
        id: &ConversationId,
        progress: &mut dyn FnMut(super::CompactionProgress),
    ) -> Result<super::CompactionResult, String> {
        self.compact_cancellable(id, progress, None)
    }
    pub fn compact_cancellable(
        &self,
        id: &ConversationId,
        progress: &mut dyn FnMut(super::CompactionProgress),
        cancelled: Option<&std::sync::atomic::AtomicBool>,
    ) -> Result<super::CompactionResult, String> {
        let _lease = self.store.acquire_execution(id)?;
        let agent = self.runtime.default_agent()?;
        let policy = agent
            .memory_policy()
            .ok_or("Conversation compaction is not enabled for this agent")?
            .clone();
        let mut context = crate::runtime::context::ConversationContext {
            store: self.store,
            conversation: Some(self.store.load(id)?),
            policy,
            cancelled,
        };
        context.compact(
            agent,
            crate::runtime::context::ContextPreparationRequest {
                input: "",
                tools: &self.runtime.tool_definitions(),
                images: &[],
                latest_usage: None,
            },
            super::CompactionTrigger::Manual,
            &mut |event| {
                if let ToolEvent::Compaction { progress: p } = event {
                    progress(p.clone())
                }
            },
        )
    }

    pub fn ask(
        &self,
        prompt: &str,
        id: Option<&ConversationId>,
        on_delta: &mut dyn FnMut(&str),
    ) -> Result<ConversationAskResult, String> {
        self.ask_with_workspace_and_permission(prompt, id, None, PermissionMode::Ask, on_delta)
    }

    pub fn ask_with_workspace(
        &self,
        prompt: &str,
        id: Option<&ConversationId>,
        new_workspace: Option<&Path>,
        on_delta: &mut dyn FnMut(&str),
    ) -> Result<ConversationAskResult, String> {
        self.ask_with_workspace_and_permission(
            prompt,
            id,
            new_workspace,
            PermissionMode::Ask,
            on_delta,
        )
    }

    pub fn ask_with_workspace_and_permission(
        &self,
        prompt: &str,
        id: Option<&ConversationId>,
        new_workspace: Option<&Path>,
        permission_mode: PermissionMode,
        on_delta: &mut dyn FnMut(&str),
    ) -> Result<ConversationAskResult, String> {
        self.ask_with_workspace_permission_and_fallible_sink(
            prompt,
            id,
            new_workspace,
            permission_mode,
            &mut |delta| {
                on_delta(delta);
                Ok(())
            },
        )
    }

    pub fn ask_with_fallible_sink(
        &self,
        prompt: &str,
        id: Option<&ConversationId>,
        on_delta: &mut dyn FnMut(&str) -> Result<(), String>,
    ) -> Result<ConversationAskResult, String> {
        self.ask_with_workspace_permission_and_fallible_sink(
            prompt,
            id,
            None,
            PermissionMode::Ask,
            on_delta,
        )
    }

    pub fn ask_with_workspace_and_fallible_sink(
        &self,
        prompt: &str,
        id: Option<&ConversationId>,
        new_workspace: Option<&Path>,
        on_delta: &mut dyn FnMut(&str) -> Result<(), String>,
    ) -> Result<ConversationAskResult, String> {
        self.ask_with_workspace_permission_and_fallible_sink(
            prompt,
            id,
            new_workspace,
            PermissionMode::Ask,
            on_delta,
        )
    }

    pub fn ask_with_workspace_permission_and_fallible_sink(
        &self,
        prompt: &str,
        id: Option<&ConversationId>,
        new_workspace: Option<&Path>,
        permission_mode: PermissionMode,
        on_delta: &mut dyn FnMut(&str) -> Result<(), String>,
    ) -> Result<ConversationAskResult, String> {
        let mut on_tool_event = |_: &ToolEvent| {};
        self.ask_with_workspace_permission_authorizer_and_fallible_sink(
            prompt,
            id,
            new_workspace,
            permission_mode,
            AskCallbacks {
                authorizer: &MissingAuthorizer,
                on_delta,
                on_tool_event: &mut on_tool_event,
            },
        )
    }

    pub fn ask_with_permission_authorizer(
        &self,
        prompt: &str,
        id: Option<&ConversationId>,
        permission_mode: PermissionMode,
        authorizer: &dyn ToolAuthorizer,
        on_delta: &mut dyn FnMut(&str),
        on_tool_event: &mut dyn FnMut(&ToolEvent),
    ) -> Result<ConversationAskResult, String> {
        let mut fallible_delta = |delta: &str| {
            on_delta(delta);
            Ok(())
        };
        self.ask_with_workspace_permission_authorizer_and_fallible_sink(
            prompt,
            id,
            None,
            permission_mode,
            AskCallbacks {
                authorizer,
                on_delta: &mut fallible_delta,
                on_tool_event,
            },
        )
    }

    pub fn ask_with_workspace_permission_authorizer_and_fallible_sink(
        &self,
        prompt: &str,
        id: Option<&ConversationId>,
        new_workspace: Option<&Path>,
        requested_permission_mode: PermissionMode,
        callbacks: AskCallbacks<'_>,
    ) -> Result<ConversationAskResult, String> {
        let _lease = id.map(|id| self.store.acquire_execution(id)).transpose()?;
        let config = crate::observability::ObservabilityConfig::load()?;
        let first = id
            .map(|id| {
                self.store
                    .load(id)
                    .map(|c| config.session.at(c.metadata.created_at))
            })
            .transpose()?
            .unwrap_or_else(|| config.session.now());
        let sessions = crate::sessions::SessionStore::new(self.store.data_root(), config.session)?;
        let journal = sessions.open(id, self.session_key, first)?;
        let turn = journal.start_turn(prompt)?;
        let _trace = crate::sessions::trace::enter(crate::sessions::trace::Trace::root(
            journal.clone(),
            &turn,
        ));
        let mut result = self.ask_recorded(
            prompt,
            id,
            new_workspace,
            requested_permission_mode,
            callbacks,
            &journal,
            &turn,
        );
        let status = match &result {
            Ok(result) if result.interrupted => "stopped",
            Ok(_) => "succeeded",
            Err(_) => "failed",
        };
        if let Err(error) = &result {
            crate::diagnostics::event(
                "ERROR",
                "session.turn_failed",
                serde_json::json!({"sessionId":journal.id(),"turnId":turn,"error":error}),
            );
        }
        if let Err(error) = journal.finish_turn(&turn, status) {
            match &mut result {
                Ok(committed) => committed.warn_after_commit(&error),
                Err(_) => {
                    return Err(format!(
                        "Session journal could not finish this turn: {error}"
                    ))
                }
            }
        }
        result
    }

    #[allow(clippy::too_many_arguments)] // A single native ask plus its owned journal/turn context.
    fn ask_recorded(
        &self,
        prompt: &str,
        id: Option<&ConversationId>,
        new_workspace: Option<&Path>,
        requested_permission_mode: PermissionMode,
        callbacks: AskCallbacks<'_>,
        journal: &crate::sessions::SessionJournal,
        turn: &str,
    ) -> Result<ConversationAskResult, String> {
        let agent = self.runtime.default_agent()?;
        let policy = agent
            .memory_policy()
            .cloned()
            .filter(|_| std::env::var_os("ATLAS_HISTORY_MESSAGES").is_none());
        if policy.is_none() && agent.memory_policy().is_some() {
            (callbacks.on_tool_event)(&ToolEvent::ContextStatus {
                status: crate::runtime::context::ContextStatus {
                    available: false,
                    message:
                        "Legacy history mode: ATLAS_HISTORY_MESSAGES overrides full-history memory"
                            .into(),
                    estimated: false,
                    input_tokens: None,
                    max_tokens: None,
                },
            });
        }
        let mut managed = crate::runtime::context::ConversationContext {
            store: self.store,
            conversation: id.map(|id| self.store.load(id)).transpose()?,
            policy: policy.clone().unwrap_or_default(),
            cancelled: self.cancelled,
        };
        let (history, activity_context, permission_mode) = match id {
            Some(id) => {
                let conversation = self.store.load(id)?;
                let permission_mode = conversation
                    .metadata
                    .permission_mode
                    .unwrap_or(PermissionMode::Ask);
                let mut history = select_recent_history(&conversation.messages, self.history_limit);
                let first_message_index = conversation.messages.len().saturating_sub(history.len());
                let activities = conversation
                    .activities
                    .into_iter()
                    .filter(|turn| turn.assistant_message_index >= first_message_index)
                    .collect::<Vec<_>>();
                super::annotate_interrupted_history(&mut history, first_message_index, &activities);
                let activity_context = render_activity_context(&activities);
                (history, activity_context, permission_mode)
            }
            None => (Vec::new(), String::new(), requested_permission_mode),
        };
        journal.record(turn,"turn_context",serde_json::json!({
            "workspacePath":managed.conversation.as_ref().and_then(|c|c.metadata.workspace_path.as_deref()).or(new_workspace),
            "permissionMode":permission_mode,"model":agent.model_name(),"agent":agent.name(),
            "historyMode":if policy.is_some(){"managed"}else{"bounded"},
            "applicationVersion":env!("CARGO_PKG_VERSION")
        }))?;
        let mut sink_error = None;
        let mut collector = ActivityCollector::new();
        let mut latest_usage = None;
        let displayed_answer = std::cell::RefCell::new(String::new());
        let mut capture_event = |event: &ToolEvent| {
            if matches!(event, ToolEvent::ResponseReset) {
                let content = displayed_answer.borrow();
                if !content.is_empty() {
                    let _ = journal.record(
                        turn,
                        "assistant",
                        serde_json::json!({
                            "content":&*content,"status":"superseded"
                        }),
                    );
                }
            }
            let _ = journal.record_runtime_event(turn, event);
            match event {
                ToolEvent::Progress { message } => collector.progress(message),
                ToolEvent::Usage { usage } => latest_usage = usage.clone(),
                ToolEvent::Status { status } => collector.status(match status {
                    RuntimeStatus::Thinking => "Thinking",
                    RuntimeStatus::PreparingAnswer => "Preparing answer",
                }),
                ToolEvent::Requested {
                    tool, arguments, ..
                } => collector.tool_requested_sanitized(tool, arguments.clone()),
                ToolEvent::Denied { tool, .. } => collector.tool_denied(tool),
                ToolEvent::Finished {
                    tool,
                    success,
                    summary,
                    details,
                    ..
                } => collector.tool_finished(
                    tool,
                    *success,
                    summary,
                    details.as_deref().unwrap_or_default(),
                ),
                ToolEvent::ResponseReset => {
                    self.runtime.reset_displayed_output();
                    displayed_answer.borrow_mut().clear();
                }
                ToolEvent::ProgressPreview { .. }
                | ToolEvent::RequestStarted { .. }
                | ToolEvent::ModelState { .. }
                | ToolEvent::UserInputAccepted { .. }
                | ToolEvent::ApprovalRequested { .. }
                | ToolEvent::Started { .. }
                | ToolEvent::Compaction { .. }
                | ToolEvent::ContextStatus { .. } => {}
            }
            (callbacks.on_tool_event)(event);
        };
        let activity_started_at = Instant::now();
        let mut static_context = crate::runtime::context::StaticContext {
            history: &history,
            activity_context: (!activity_context.is_empty()).then_some(activity_context.as_str()),
        };
        // A direct child budgets its frozen snapshot using its own provider. Do not
        // run Atlas compaction inference while preparing the native dispatch.
        let mut direct_snapshot = (self.runtime.is_direct_agent() && policy.is_some())
            .then(|| crate::runtime::context::SnapshotContext(managed.prepared()));
        let context: &mut dyn crate::runtime::context::RuntimeContext =
            if let Some(snapshot) = direct_snapshot.as_mut() {
                snapshot
            } else if policy.is_some() {
                &mut managed
            } else {
                &mut static_context
            };
        let mut context = crate::runtime::context::CancellableContext {
            inner: context,
            cancelled: self.cancelled,
        };
        let mut context = crate::sessions::context::SessionContext {
            inner: &mut context,
            journal,
            turn,
            store: self.store,
            conversation_id: id,
        };
        let answer = self.runtime.ask_stream_with_managed_context(
            prompt,
            &mut context,
            permission_mode,
            RuntimeAskCallbacks {
                authorizer: callbacks.authorizer,
                on_delta: &mut |delta| {
                    if sink_error.is_none() {
                        sink_error = (callbacks.on_delta)(delta).err();
                        if sink_error.is_none() {
                            self.runtime.record_displayed_delta(delta);
                            displayed_answer.borrow_mut().push_str(delta);
                        }
                    }
                },
                on_tool_event: &mut capture_event,
            },
        );
        let interrupted = self.runtime.user_stop_requested()
            && self
                .cancelled
                .is_some_and(|cancelled| cancelled.load(std::sync::atomic::Ordering::Relaxed));
        let content = if interrupted {
            self.runtime
                .stopped_prefix()?
                .unwrap_or_else(|| displayed_answer.borrow().clone())
        } else if sink_error.is_none() {
            answer
                .as_ref()
                .cloned()
                .unwrap_or_else(|_| displayed_answer.borrow().clone())
        } else {
            displayed_answer.borrow().clone()
        };
        let status = if sink_error.is_some() {
            "failed"
        } else if interrupted {
            "stopped"
        } else if answer.is_ok() {
            "succeeded"
        } else {
            "failed"
        };
        if !content.is_empty() || interrupted || (answer.is_ok() && sink_error.is_none()) {
            journal.record(
                turn,
                "assistant",
                serde_json::json!({"content":content,"status":status}),
            )?;
        }
        journal.check()?;
        if let Some(error) = sink_error {
            return Err(error);
        }
        let answer = if interrupted { content } else { answer? };
        let additional_inputs = self.runtime.applied_user_inputs();
        let activity_duration_ms =
            u64::try_from(activity_started_at.elapsed().as_millis()).unwrap_or(u64::MAX);
        let mut activity_turn = collector.into_unassociated_turn();
        activity_turn.duration_ms = Some(activity_duration_ms);
        activity_turn.interrupted = interrupted;
        activity_turn.usage = latest_usage.clone();
        let activities = activity_turn.records.clone();

        let mut post_commit_error = None;
        let metadata = match id {
            Some(id) => self.store.append_turn_with_inputs_and_activity(
                id,
                prompt,
                &additional_inputs,
                &answer,
                new_workspace,
                Some(permission_mode),
                activity_turn,
            )?,
            None => {
                let metadata = self.store.create_session_turn(
                    &journal.metadata()?,
                    prompt,
                    &additional_inputs,
                    &answer,
                    (new_workspace, permission_mode),
                    activity_turn,
                )?;
                let title = if interrupted {
                    None
                } else if journal
                    .record(
                        turn,
                        "title_generation_started",
                        serde_json::json!({"model":agent.model_name()}),
                    )
                    .is_ok()
                {
                    let response = self.runtime.generate_conversation_title(prompt);
                    let _=journal.record(turn,"title_generation_finished",serde_json::json!({"status":if response.is_ok(){"succeeded"}else{"failed"}}));
                    response.ok().and_then(|raw| sanitize_title(&raw))
                } else {
                    None
                };
                match self
                    .store
                    .record_title_attempt(&metadata.id, title.as_deref())
                {
                    Ok(updated) => updated,
                    Err(error) => {
                        post_commit_error = Some(error);
                        metadata
                    }
                }
            }
        };

        let finalized = (|| {
            journal.record(turn,"conversation_committed",serde_json::json!({"conversationId":metadata.id,"messageCount":metadata.message_count}))?;
            Ok::<(), String>(())
        })();
        if let Err(error) = finalized {
            post_commit_error = Some(error);
        }
        let memory = match self.store.load_memory(&metadata.id) {
            Ok(memory) => memory,
            Err(error) => {
                post_commit_error = Some(error);
                None
            }
        };
        let mut result = ConversationAskResult {
            memory,
            answer,
            additional_inputs,
            interrupted,
            metadata,
            activities,
            activity_duration_ms,
            usage: latest_usage,
            created: id.is_none(),
            persistence_warning: None,
        };
        if let Some(error) = post_commit_error {
            result.warn_after_commit(&error);
        }
        Ok(result)
    }
}
