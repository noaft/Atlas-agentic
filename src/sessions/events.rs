use super::SessionJournal;
use crate::permissions::ToolEvent;
use serde_json::json;
impl SessionJournal {
    pub fn record_runtime_event(&self, turn: &str, event: &ToolEvent) -> Result<(), String> {
        self.record_runtime_event_scoped(turn, event, None)
    }
    pub(crate) fn record_runtime_event_scoped(
        &self,
        turn: &str,
        event: &ToolEvent,
        run: Option<&str>,
    ) -> Result<(), String> {
        let kind = match event {
            ToolEvent::RequestStarted { .. } => "request_started",
            ToolEvent::ModelState { .. } => "model_state",
            ToolEvent::UserInputAccepted { .. } => "user_input_accepted",
            ToolEvent::ProgressPreview { message } => {
                self.record(turn,"progress_preview",json!({"present":message.is_some(),"characters":message.as_ref().map_or(0,|s|s.chars().count()),"agentRunId":run}))?;
                return Ok(());
            }
            ToolEvent::Progress { .. } => "progress",
            ToolEvent::ResponseReset => "response_reset",
            ToolEvent::Compaction { .. } => "compaction_progress",
            ToolEvent::ContextStatus { .. } => "context_status",
            ToolEvent::Usage { .. } => "model_usage",
            ToolEvent::Status { .. } => "runtime_status",
            ToolEvent::Requested { .. } => "tool_call",
            ToolEvent::ApprovalRequested { .. } => "approval_requested",
            ToolEvent::Denied { .. } => "tool_denied",
            ToolEvent::Started { .. } => "tool_started",
            ToolEvent::Finished { .. } => "tool_result",
        };
        let mut payload =
            serde_json::to_value(event).map_err(|_| "Could not encode session runtime event")?;
        let mut data = payload
            .get_mut("data")
            .map(serde_json::Value::take)
            .unwrap_or(serde_json::Value::Null);
        if let Some(run) = run {
            if !data.is_object() {
                data = json!({});
            }
            data["agentRunId"] = json!(run);
        }
        self.record(turn, kind, data)?;
        Ok(())
    }
}
