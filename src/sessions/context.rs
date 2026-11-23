//! Trace context selection without using trace events as model instructions.
use super::{ContextCursor, SessionJournal};
use crate::{
    agents::Agent,
    conversations::{ConversationId, ConversationStore},
    permissions::ToolEvent,
    runtime::context::{ContextPreparationRequest, PreparedContext, RuntimeContext},
};
use serde_json::json;
use sha2::{Digest, Sha256};
pub(crate) struct SessionContext<'a> {
    pub inner: &'a mut dyn RuntimeContext,
    pub journal: &'a SessionJournal,
    pub turn: &'a str,
    pub store: &'a ConversationStore,
    pub conversation_id: Option<&'a ConversationId>,
}
impl RuntimeContext for SessionContext<'_> {
    fn check_cancelled(&self) -> Result<(), String> {
        self.journal.check()?;
        self.inner.check_cancelled()
    }
    fn record_step(&self, kind: &str, data: serde_json::Value) -> Result<(), String> {
        self.journal.record(self.turn, kind, data)?;
        Ok(())
    }
    fn prepare(
        &mut self,
        agent: &dyn Agent,
        request: ContextPreparationRequest<'_>,
        events: &mut dyn FnMut(&ToolEvent),
    ) -> Result<PreparedContext, String> {
        self.prepare_with_control(agent, request, events, &|| false)
    }
    fn prepare_with_control(
        &mut self,
        agent: &dyn Agent,
        request: ContextPreparationRequest<'_>,
        events: &mut dyn FnMut(&ToolEvent),
        interrupted: &dyn Fn() -> bool,
    ) -> Result<PreparedContext, String> {
        self.check_cancelled()?;
        let prepared = self.inner.prepare_with_control(
            agent,
            ContextPreparationRequest { ..request },
            events,
            &|| interrupted() || self.journal.check().is_err(),
        )?;
        self.check_cancelled()?;
        let memory = self
            .conversation_id
            .map(|id| self.store.load_memory(id))
            .transpose()?
            .flatten();
        let start = memory
            .as_ref()
            .filter(|_| !prepared.memory_blocks.is_empty())
            .map(|m| m.compacted_message_count)
            .unwrap_or(
                self.conversation_id
                    .map(|id| {
                        self.store
                            .load(id)
                            .map(|c| c.messages.len().saturating_sub(prepared.history.len()))
                    })
                    .transpose()?
                    .unwrap_or(0),
            );
        let cursor = ContextCursor {
            history_message_start: start,
            checkpoint_id: memory
                .filter(|_| !prepared.memory_blocks.is_empty())
                .map(|m| m.checkpoint_id),
            prepared_event: None,
        };
        let tools = request
            .tools
            .iter()
            .map(|t| json!({"name":t.name,"description":t.description,"schema":t.input_schema}))
            .collect::<Vec<_>>();
        let digest = |bytes: &[u8]| format!("{:x}", Sha256::digest(bytes));
        self.journal.prepare_context(self.turn,cursor,json!({
            "source":"committed_conversation_and_current_continuation",
            "conversationId":self.conversation_id,
            "historyMessages":prepared.history.len(),
            "historyFingerprint":crate::conversations::memory_fingerprint(&prepared.history)?,
            "memoryRanges":prepared.memory_blocks.iter().map(|b|json!({"start":b.start_index,"endExclusive":b.end_index_exclusive})).collect::<Vec<_>>(),
            "input":request.input,"inputSha256":digest(request.input.as_bytes()),
            "toolNames":request.tools.iter().map(|t|&t.name).collect::<Vec<_>>(),
            "toolCatalogSha256":digest(&serde_json::to_vec(&tools).map_err(|_|"Could not fingerprint session tool catalog")?),
            "imageCount":request.images.len(),"model":agent.model_name(),"maxOutputTokens":prepared.max_output_tokens
        }))?;
        Ok(prepared)
    }
}
