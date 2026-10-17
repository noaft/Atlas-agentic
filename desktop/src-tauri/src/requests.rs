use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
};

use atlas_agentic::conversations::ConversationId;

#[derive(Default)]
pub(crate) struct ActiveRequests {
    conversations: Mutex<HashMap<ConversationId, Arc<AtomicBool>>>,
}

impl ActiveRequests {
    pub(crate) fn acquire(
        self: &Arc<Self>,
        id: Option<&ConversationId>,
    ) -> Result<RequestGuard, String> {
        let cancelled = Arc::new(AtomicBool::new(false));
        if let Some(id) = id {
            let mut conversations = self
                .conversations
                .lock()
                .map_err(|_| "active request registry lock was poisoned".to_string())?;
            if conversations.contains_key(id) {
                return Err(format!(
                    "a request is already running for conversation {id}"
                ));
            }
            conversations.insert(id.clone(), cancelled.clone());
        }
        Ok(RequestGuard {
            requests: Arc::clone(self),
            id: id.cloned(),
            cancelled,
        })
    }
    pub(crate) fn cancel(&self, id: &ConversationId) -> Result<(), String> {
        if let Some(flag) = self
            .conversations
            .lock()
            .map_err(|_| "Request lock poisoned")?
            .get(id)
        {
            flag.store(true, Ordering::Relaxed);
        }
        Ok(())
    }
}

pub(crate) struct RequestGuard {
    pub(crate) cancelled: Arc<AtomicBool>,
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

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cancellation_reaches_only_the_selected_conversation_guard() {
        let requests = Arc::new(ActiveRequests::default());
        let a = ConversationId::new();
        let b = ConversationId::new();
        let guard_a = requests.acquire(Some(&a)).unwrap();
        let guard_b = requests.acquire(Some(&b)).unwrap();
        requests.cancel(&a).unwrap();
        assert!(guard_a.cancelled.load(Ordering::Relaxed));
        assert!(!guard_b.cancelled.load(Ordering::Relaxed));
    }
}
