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
    pub metadata: ConversationMetadata,
    pub activities: Vec<ActivityRecord>,
    pub activity_duration_ms: u64,
    pub created: bool,
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
}

impl<'a> ConversationService<'a> {
    pub fn new(runtime: &'a Runtime, store: &'a ConversationStore, history_limit: usize) -> Self {
        Self {
            runtime,
            store,
            history_limit,
        }
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
        let (history, activity_context, permission_mode) = match id {
            Some(id) => {
                let conversation = self.store.load(id)?;
                let permission_mode = conversation
                    .metadata
                    .permission_mode
                    .unwrap_or(PermissionMode::Ask);
                let history = select_recent_history(&conversation.messages, self.history_limit);
                let first_message_index = conversation.messages.len().saturating_sub(history.len());
                let activities = conversation
                    .activities
                    .into_iter()
                    .filter(|turn| turn.assistant_message_index >= first_message_index)
                    .collect::<Vec<_>>();
                let activity_context = render_activity_context(&activities);
                (history, activity_context, permission_mode)
            }
            None => (Vec::new(), String::new(), requested_permission_mode),
        };
        let mut sink_error = None;
        let mut collector = ActivityCollector::new();
        let mut capture_event = |event: &ToolEvent| {
            match event {
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
                ToolEvent::ApprovalRequested { .. } | ToolEvent::Started { .. } => {}
            }
            (callbacks.on_tool_event)(event);
        };
        let activity_started_at = Instant::now();
        let answer = self.runtime.ask_stream_with_context_and_authorization(
            prompt,
            &history,
            (!activity_context.is_empty()).then_some(activity_context.as_str()),
            permission_mode,
            RuntimeAskCallbacks {
                authorizer: callbacks.authorizer,
                on_delta: &mut |delta| {
                    if sink_error.is_none() {
                        sink_error = (callbacks.on_delta)(delta).err();
                    }
                },
                on_tool_event: &mut capture_event,
            },
        );
        if let Some(error) = sink_error {
            return Err(error);
        }
        let answer = answer?;
        let activity_duration_ms =
            u64::try_from(activity_started_at.elapsed().as_millis()).unwrap_or(u64::MAX);
        let mut activity_turn = collector.into_unassociated_turn();
        activity_turn.duration_ms = Some(activity_duration_ms);
        let activities = activity_turn.records.clone();

        let metadata = match id {
            Some(id) => self.store.append_turn_with_activity(
                id,
                prompt,
                &answer,
                new_workspace,
                Some(permission_mode),
                activity_turn,
            )?,
            None => {
                let metadata = self.store.create_turn_with_activity(
                    prompt,
                    &answer,
                    new_workspace,
                    Some(permission_mode),
                    activity_turn,
                )?;
                let title = self
                    .runtime
                    .generate_conversation_title(prompt)
                    .ok()
                    .and_then(|raw| sanitize_title(&raw));
                self.store
                    .record_title_attempt(&metadata.id, title.as_deref())?
            }
        };

        Ok(ConversationAskResult {
            answer,
            metadata,
            activities,
            activity_duration_ms,
            created: id.is_none(),
        })
    }
}
