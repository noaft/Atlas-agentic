use std::{
    collections::HashSet,
    sync::{Arc, Mutex},
};

use atlas_agentic::conversations::ConversationId;

#[derive(Default)]
pub(crate) struct ActiveRequests {
    conversations: Mutex<HashSet<ConversationId>>,
}

impl ActiveRequests {
    pub(crate) fn acquire(
        self: &Arc<Self>,
        id: Option<&ConversationId>,
    ) -> Result<RequestGuard, String> {
        if let Some(id) = id {
            let mut conversations = self
                .conversations
                .lock()
                .map_err(|_| "active request registry lock was poisoned".to_string())?;
            if !conversations.insert(id.clone()) {
                return Err(format!(
                    "a request is already running for conversation {id}"
                ));
            }
        }
        Ok(RequestGuard {
            requests: Arc::clone(self),
            id: id.cloned(),
        })
    }
}

pub(crate) struct RequestGuard {
    requests: Arc<ActiveRequests>,
    id: Option<ConversationId>,
}

impl Drop for RequestGuard {
    fn drop(&mut self) {
        if let Some(id) = &self.id {
            if let Ok(mut conversations) = self.requests.conversations.lock() {
                conversations.remove(id);
            }
        }
    }
}
