use crate::Runtime;

use super::{
    sanitize_title, select_recent_history, ConversationId, ConversationMetadata, ConversationStore,
};

#[derive(Debug)]
pub struct ConversationAskResult {
    pub answer: String,
    pub metadata: ConversationMetadata,
    pub created: bool,
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
        self.ask_with_fallible_sink(prompt, id, &mut |delta| {
            on_delta(delta);
            Ok(())
        })
    }

    pub fn ask_with_fallible_sink(
        &self,
        prompt: &str,
        id: Option<&ConversationId>,
        on_delta: &mut dyn FnMut(&str) -> Result<(), String>,
    ) -> Result<ConversationAskResult, String> {
        let history = match id {
            Some(id) => {
                let conversation = self.store.load(id)?;
                select_recent_history(&conversation.messages, self.history_limit)
            }
            None => Vec::new(),
        };
        let mut sink_error = None;
        let answer = self
            .runtime
            .ask_stream_with_history(prompt, &history, &mut |delta| {
                if sink_error.is_none() {
                    sink_error = on_delta(delta).err();
                }
            });
        if let Some(error) = sink_error {
            return Err(error);
        }
        let answer = answer?;

        let metadata = match id {
            Some(id) => self.store.append_turn(id, prompt, &answer)?,
            None => {
                let metadata = self.store.create_turn(prompt, &answer)?;
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
            created: id.is_none(),
        })
    }
}
