//! Native ownership and lifecycle for the app's in-memory process resources.
use crate::commands::{run_blocking, DesktopState};
use atlas_agentic::{
    conversations::ConversationId,
    process_limits::ProcessLimits,
    tools::{ProcessCursor, ProcessInfo, ProcessManager, ProcessRead, ProcessScope},
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{HashMap, HashSet},
    sync::{Arc, Mutex},
    time::Duration,
};
use tauri::{State, WebviewWindow};
#[derive(Clone, Default)]
pub(crate) struct DesktopProcesses {
    manager: ProcessManager,
    registry: Arc<Mutex<Registry>>,
}
#[derive(Default)]
struct Registry {
    clients: HashMap<String, String>,
    drafts: HashMap<String, (String, String, ProcessScope)>,
    saved: HashMap<ConversationId, ProcessScope>,
    active: HashSet<String>,
    promoted: HashMap<String, (String, String, ConversationId)>,
    closing: bool,
}
#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProcessContext {
    pub conversation_id: Option<ConversationId>,
    pub scope_id: Option<String>,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScopeView {
    pub scope_id: String,
}
pub(crate) struct ProcessLease {
    registry: Arc<Mutex<Registry>>,
    id: String,
}
impl Drop for ProcessLease {
    fn drop(&mut self) {
        self.registry
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .active
            .remove(&self.id);
    }
}
impl DesktopProcesses {
    pub(crate) fn set_limits(&self, limits: ProcessLimits) -> Result<(), String> {
        self.manager.set_limits(limits)
    }
    pub(crate) fn initialize(&self, window: &str) -> Result<String, String> {
        let token = uuid();
        let old = {
            let mut registry = self
                .registry
                .lock()
                .map_err(|_| "Process ownership unavailable")?;
            if registry.closing {
                return Err("Process manager is shutting down".into());
            }
            registry.clients.insert(window.into(), token.clone());
            registry.promoted.retain(|_, entry| entry.0 != window);
            let ids = registry
                .drafts
                .iter()
                .filter(|(_, entry)| entry.0 == window)
                .map(|(id, _)| id.clone())
                .collect::<Vec<_>>();
            ids.into_iter()
                .filter_map(|id| registry.drafts.remove(&id).map(|e| e.2))
                .collect::<Vec<_>>()
        };
        for scope in old {
            scope.close(Duration::from_secs(3))?;
        }
        Ok(token)
    }
    pub(crate) fn resolve(
        &self,
        window: &str,
        client: &str,
        context: &ProcessContext,
    ) -> Result<ProcessScope, String> {
        let mut registry = self
            .registry
            .lock()
            .map_err(|_| "Process ownership unavailable")?;
        if registry.closing || registry.clients.get(window).map(String::as_str) != Some(client) {
            return Err("Process client expired; reload process controls".into());
        }
        if let Some(id) = &context.conversation_id {
            return match registry.saved.get(id) {
                Some(scope) => Ok(scope.clone()),
                None => {
                    let scope = self.manager.create_scope()?;
                    registry.saved.insert(id.clone(), scope.clone());
                    Ok(scope)
                }
            };
        }
        if let Some(id) = &context.scope_id {
            if let Some(entry) = registry
                .drafts
                .get(id)
                .filter(|entry| entry.0 == window && entry.1 == client)
            {
                return Ok(entry.2.clone());
            }
            if let Some(entry) = registry
                .promoted
                .get(id)
                .filter(|entry| entry.0 == window && entry.1 == client)
            {
                return registry
                    .saved
                    .get(&entry.2)
                    .cloned()
                    .ok_or("Saved process scope no longer exists".into());
            }
            return Err("Draft process scope does not belong to this client".into());
        }
        if registry
            .drafts
            .values()
            .filter(|entry| entry.0 == window)
            .count()
            >= 64
        {
            return Err("Too many draft process scopes; close unused drafts".into());
        }
        let scope = self.manager.create_scope()?;
        registry.drafts.insert(
            scope.id().into(),
            (window.into(), client.into(), scope.clone()),
        );
        Ok(scope)
    }
    pub(crate) fn request_context(
        &self,
        window: &str,
        client: &str,
        context: ProcessContext,
    ) -> Result<ProcessContext, String> {
        let registry = self
            .registry
            .lock()
            .map_err(|_| "Process ownership unavailable")?;
        if registry.closing || registry.clients.get(window).map(String::as_str) != Some(client) {
            return Err("Process client expired".into());
        }
        if context.conversation_id.is_some() {
            return Ok(context);
        }
        let scope_id = context
            .scope_id
            .as_deref()
            .ok_or("Open a draft process scope before sending a request")?;
        if let Some(entry) = registry
            .promoted
            .get(scope_id)
            .filter(|entry| entry.0 == window && entry.1 == client)
        {
            return Ok(ProcessContext {
                conversation_id: Some(entry.2.clone()),
                scope_id: context.scope_id,
            });
        }
        Ok(context)
    }
    pub(crate) fn lease(&self, scope: &ProcessScope) -> Result<ProcessLease, String> {
        let mut registry = self
            .registry
            .lock()
            .map_err(|_| "Process ownership unavailable")?;
        if !registry.active.insert(scope.id().into()) {
            return Err("A request is already running for this process scope".into());
        }
        Ok(ProcessLease {
            registry: self.registry.clone(),
            id: scope.id().into(),
        })
    }
    pub(crate) fn bind(&self, scope: &ProcessScope, id: &ConversationId) -> Result<(), String> {
        let mut registry = self
            .registry
            .lock()
            .map_err(|_| "Process ownership unavailable")?;
        if registry.closing {
            return Ok(());
        }
        let Some((window, client, _)) = registry.drafts.get(scope.id()).cloned() else {
            return Ok(());
        };
        let target = registry
            .saved
            .get(id)
            .cloned()
            .unwrap_or_else(|| scope.clone());
        if target.id() != scope.id() && target.adopt_from(scope).is_err() {
            registry.drafts.remove(scope.id());
            drop(registry);
            let _ = scope.close(Duration::from_secs(3));
            return Err("Background processes could not be transferred to the saved chat".into());
        }
        registry.saved.insert(id.clone(), target);
        registry.drafts.remove(scope.id());
        registry
            .promoted
            .insert(scope.id().into(), (window, client, id.clone()));
        Ok(())
    }
    pub(crate) fn release(
        &self,
        window: &str,
        client: &str,
        context: &ProcessContext,
    ) -> Result<(), String> {
        if context.conversation_id.is_some() {
            return Err("Saved process scopes are closed through conversation deletion".into());
        }
        let id = context
            .scope_id
            .as_deref()
            .ok_or("Draft scope ID is required")?;
        let scope = {
            let mut registry = self
                .registry
                .lock()
                .map_err(|_| "Process ownership unavailable")?;
            if registry.clients.get(window).map(String::as_str) != Some(client) {
                return Err("Process client expired".into());
            }
            if registry.active.contains(id) {
                return Err("A request is still running in this draft".into());
            }
            let entry = registry
                .drafts
                .get(id)
                .filter(|entry| entry.0 == window && entry.1 == client)
                .ok_or("Draft process scope does not belong to this client")?;
            let scope = entry.2.clone();
            registry.drafts.remove(id);
            scope
        };
        scope.close(Duration::from_secs(3))
    }
    pub(crate) fn delete(&self, id: &ConversationId) -> Result<(), String> {
        let scope = self
            .registry
            .lock()
            .map_err(|_| "Process ownership unavailable")?
            .saved
            .get(id)
            .cloned();
        if let Some(scope) = scope {
            scope.close(Duration::from_secs(3))?;
            let mut registry = self
                .registry
                .lock()
                .map_err(|_| "Process ownership unavailable")?;
            registry.saved.remove(id);
            registry.promoted.retain(|_, entry| &entry.2 != id);
        }
        Ok(())
    }
    pub(crate) fn shutdown(&self) -> Result<(), String> {
        self.registry
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .closing = true;
        self.manager.shutdown(Duration::from_secs(3))
    }
}
fn uuid() -> String {
    atlas_agentic::conversations::ConversationId::new().to_string()
}
fn resolve(
    state: &DesktopState,
    window: &str,
    client: &str,
    context: &ProcessContext,
) -> Result<ProcessScope, String> {
    state.validate_process_conversation(context.conversation_id.as_ref())?;
    state.processes.resolve(window, client, context)
}
#[tauri::command]
pub async fn initialize_process_client(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
) -> Result<String, String> {
    let processes = state.processes.clone();
    let label = window.label().to_string();
    run_blocking("desktop.process_client", move || {
        processes.initialize(&label)
    })
    .await
}
#[tauri::command]
pub async fn open_process_scope(
    client: String,
    context: ProcessContext,
    window: WebviewWindow,
    state: State<'_, DesktopState>,
) -> Result<ScopeView, String> {
    let state = state.inner().clone();
    let label = window.label().to_string();
    run_blocking("desktop.process_scope", move || {
        Ok(ScopeView {
            scope_id: resolve(&state, &label, &client, &context)?.id().into(),
        })
    })
    .await
}
#[tauri::command]
pub async fn release_process_scope(
    client: String,
    context: ProcessContext,
    window: WebviewWindow,
    state: State<'_, DesktopState>,
) -> Result<(), String> {
    let processes = state.processes.clone();
    let label = window.label().to_string();
    run_blocking("desktop.release_process_scope", move || {
        processes.release(&label, &client, &context)
    })
    .await
}
#[tauri::command]
pub async fn list_processes(
    client: String,
    context: ProcessContext,
    window: WebviewWindow,
    state: State<'_, DesktopState>,
) -> Result<Vec<ProcessInfo>, String> {
    let state = state.inner().clone();
    let label = window.label().to_string();
    run_blocking("desktop.list_processes", move || {
        resolve(&state, &label, &client, &context)?.list()
    })
    .await
}
#[tauri::command]
pub async fn read_process_output(
    client: String,
    context: ProcessContext,
    session_id: String,
    cursor: ProcessCursor,
    window: WebviewWindow,
    state: State<'_, DesktopState>,
) -> Result<ProcessRead, String> {
    let state = state.inner().clone();
    let label = window.label().to_string();
    run_blocking("desktop.process_output", move || {
        resolve(&state, &label, &client, &context)?.read(&session_id, Some(cursor), 0)
    })
    .await
}
#[tauri::command]
pub async fn stop_process(
    client: String,
    context: ProcessContext,
    session_id: String,
    window: WebviewWindow,
    state: State<'_, DesktopState>,
) -> Result<ProcessRead, String> {
    let state = state.inner().clone();
    let label = window.label().to_string();
    run_blocking("desktop.stop_process", move || {
        resolve(&state, &label, &client, &context)?.stop(&session_id)
    })
    .await
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn draft_tokens_transfer_and_reload_only_closes_old_drafts() {
        let hub = DesktopProcesses::default();
        let client = hub.initialize("main").unwrap();
        let context = ProcessContext {
            conversation_id: None,
            scope_id: None,
        };
        let saved = hub.resolve("main", &client, &context).unwrap();
        let draft = hub.resolve("main", &client, &context).unwrap();
        let id = ConversationId::new();
        hub.bind(&saved, &id).unwrap();
        let next = hub.initialize("main").unwrap();
        assert!(draft.list().is_err());
        let restored = hub
            .resolve(
                "main",
                &next,
                &ProcessContext {
                    conversation_id: Some(id),
                    scope_id: Some(draft.id().into()),
                },
            )
            .unwrap();
        assert_eq!(restored.id(), saved.id());
        assert!(hub.resolve("main", &client, &context).is_err());
    }
    #[test]
    fn promoted_draft_token_resolves_during_frontend_save_transition() {
        let hub = DesktopProcesses::default();
        let client = hub.initialize("main").unwrap();
        let id = ConversationId::new();
        let source = hub
            .resolve(
                "main",
                &client,
                &ProcessContext {
                    conversation_id: None,
                    scope_id: None,
                },
            )
            .unwrap();
        let placeholder = hub
            .resolve(
                "main",
                &client,
                &ProcessContext {
                    conversation_id: Some(id.clone()),
                    scope_id: None,
                },
            )
            .unwrap();
        hub.bind(&source, &id).unwrap();
        let recovered = hub
            .request_context(
                "main",
                &client,
                ProcessContext {
                    conversation_id: None,
                    scope_id: Some(source.id().into()),
                },
            )
            .unwrap();
        assert_eq!(recovered.conversation_id, Some(id.clone()));
        assert!(hub
            .request_context(
                "main",
                &client,
                ProcessContext {
                    conversation_id: None,
                    scope_id: None
                }
            )
            .is_err());
        let restored = hub
            .resolve(
                "main",
                &client,
                &ProcessContext {
                    conversation_id: None,
                    scope_id: Some(source.id().into()),
                },
            )
            .unwrap();
        assert_eq!(restored.id(), placeholder.id());
        let next = hub.initialize("other").unwrap();
        assert!(hub
            .resolve(
                "other",
                &next,
                &ProcessContext {
                    conversation_id: None,
                    scope_id: Some(source.id().into())
                }
            )
            .is_err());
    }
    #[test]
    fn released_empty_drafts_do_not_exhaust_capacity() {
        let hub = DesktopProcesses::default();
        let client = hub.initialize("main").unwrap();
        for _ in 0..80 {
            let scope = hub
                .resolve(
                    "main",
                    &client,
                    &ProcessContext {
                        conversation_id: None,
                        scope_id: None,
                    },
                )
                .unwrap();
            hub.release(
                "main",
                &client,
                &ProcessContext {
                    conversation_id: None,
                    scope_id: Some(scope.id().into()),
                },
            )
            .unwrap();
            assert!(scope.list().is_err());
        }
        assert!(hub.registry.lock().unwrap().drafts.is_empty());
    }
    #[test]
    fn ownership_and_request_leases_are_scoped() {
        let hub = DesktopProcesses::default();
        let client = hub.initialize("main").unwrap();
        let second = hub.initialize("second").unwrap();
        let draft = hub
            .resolve(
                "main",
                &client,
                &ProcessContext {
                    conversation_id: None,
                    scope_id: None,
                },
            )
            .unwrap();
        assert!(hub
            .resolve(
                "second",
                &second,
                &ProcessContext {
                    conversation_id: None,
                    scope_id: Some(draft.id().into())
                }
            )
            .is_err());
        let lease = hub.lease(&draft).unwrap();
        assert!(hub.lease(&draft).is_err());
        drop(lease);
        assert!(hub.lease(&draft).is_ok());
    }
}
