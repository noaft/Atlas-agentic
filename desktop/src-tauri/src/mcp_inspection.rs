//! Settings-owned discovery. No chat approvals or business-tool execution.
use crate::commands::DesktopState;
use atlas_agentic::{
    conversations::ConversationId,
    mcp::{inspect_server_tools, InspectedTool, McpConfigStore},
    permissions::{AuthorizationDecision, AuthorizationRequest, PermissionMode, ToolAuthorizer},
};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Arc, Mutex, OnceLock,
    },
    time::{Duration, Instant},
};
use tauri::{ipc::Channel, State};

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct InspectionContext {
    conversation_id: Option<ConversationId>,
    workspace_path: Option<PathBuf>,
    permission_mode: PermissionMode,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct InspectionRequest {
    id: String,
    server: String,
    fingerprint: String,
    context: InspectionContext,
}
#[derive(Clone, Serialize)]
#[serde(tag = "event", content = "data", rename_all = "camelCase")]
pub enum InspectionEvent {
    ApprovalRequested(AuthorizationRequest),
    Waiting,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InspectionResult {
    tools: Vec<InspectedTool>,
    workspace_path: String,
    fingerprint: String,
}
#[derive(Default)]
struct Control {
    cancelled: Arc<AtomicBool>,
    approval: Mutex<Option<(String, mpsc::Sender<AuthorizationDecision>)>>,
}
#[derive(Default)]
struct InspectionRegistry {
    active: HashMap<String, Arc<Control>>,
    early_cancellations: HashMap<String, Instant>,
}
static PENDING: OnceLock<Mutex<InspectionRegistry>> = OnceLock::new();
fn pending() -> &'static Mutex<InspectionRegistry> {
    PENDING.get_or_init(Default::default)
}
struct PendingInspection {
    id: String,
    control: Arc<Control>,
}
impl PendingInspection {
    fn reserve(id: &str) -> Result<Self, String> {
        if id.is_empty()
            || id.len() > 80
            || !id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
        {
            return Err("Invalid MCP check ID".into());
        }
        let mut all = pending()
            .lock()
            .map_err(|_| "MCP check registry unavailable")?;
        all.early_cancellations
            .retain(|_, at| at.elapsed() < Duration::from_secs(120));
        if all.early_cancellations.remove(id).is_some() {
            return Err("MCP check cancelled".into());
        }
        if all.active.len() >= 8 || all.active.contains_key(id) {
            return Err("MCP check already running or limit reached".into());
        }
        let control = Arc::new(Control::default());
        all.active.insert(id.into(), control.clone());
        Ok(Self {
            id: id.into(),
            control,
        })
    }
}
impl Drop for PendingInspection {
    fn drop(&mut self) {
        pending()
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .active
            .remove(&self.id);
    }
}
fn control(id: &str) -> Result<Arc<Control>, String> {
    pending()
        .lock()
        .map_err(|_| "MCP check registry unavailable")?
        .active
        .get(id)
        .cloned()
        .ok_or("MCP check is no longer pending".into())
}
fn cancel(id: &str) -> Result<(), String> {
    if id.is_empty() || id.len() > 80 || !id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
    {
        return Err("Invalid MCP check ID".into());
    }
    let control = {
        let mut registry = pending()
            .lock()
            .map_err(|_| "MCP check registry unavailable")?;
        match registry.active.get(id).cloned() {
            Some(control) => control,
            None => {
                // IPC cancellation can arrive before the async inspection is first polled.
                // Keep a bounded, short-lived tombstone under the same reservation lock.
                registry
                    .early_cancellations
                    .retain(|_, at| at.elapsed() < Duration::from_secs(120));
                if registry.early_cancellations.len() >= 128 {
                    if let Some(oldest) = registry
                        .early_cancellations
                        .iter()
                        .min_by_key(|(_, at)| **at)
                        .map(|(id, _)| id.clone())
                    {
                        registry.early_cancellations.remove(&oldest);
                    }
                }
                registry
                    .early_cancellations
                    .insert(id.into(), Instant::now());
                return Ok(());
            }
        }
    };
    control.cancelled.store(true, Ordering::SeqCst);
    if let Some((_, sender)) = control
        .approval
        .lock()
        .map_err(|_| "MCP approval unavailable")?
        .take()
    {
        let _ = sender.send(AuthorizationDecision::Deny);
    }
    Ok(())
}
fn resolve(id: &str, approval_id: &str, allow: bool) -> Result<(), String> {
    let control = control(id)?;
    let mut approval = control
        .approval
        .lock()
        .map_err(|_| "MCP approval unavailable")?;
    if control.cancelled.load(Ordering::SeqCst)
        || !approval.as_ref().is_some_and(|(id, _)| id == approval_id)
    {
        return Err("MCP approval is no longer pending".into());
    }
    let (_, sender) = approval.take().ok_or("MCP approval is no longer pending")?;
    sender
        .send(if allow {
            AuthorizationDecision::Allow
        } else {
            AuthorizationDecision::Deny
        })
        .map_err(|_| "MCP check is no longer waiting".into())
}
struct SettingsAuthorizer<'a> {
    control: &'a Control,
    channel: &'a Channel<InspectionEvent>,
    validate: &'a dyn Fn() -> Result<(), String>,
}
impl ToolAuthorizer for SettingsAuthorizer<'_> {
    fn authorize(&self, request: &AuthorizationRequest) -> Result<AuthorizationDecision, String> {
        let (tx, rx) = mpsc::channel();
        {
            let mut approval = self
                .control
                .approval
                .lock()
                .map_err(|_| "MCP approval unavailable")?;
            if self.control.cancelled.load(Ordering::SeqCst) {
                return Err("MCP check cancelled".into());
            }
            *approval = Some((request.id.clone(), tx));
        }
        let result = (|| {
            self.channel
                .send(InspectionEvent::ApprovalRequested(request.clone()))
                .map_err(|_| "MCP check view closed")?;
            let deadline = Instant::now() + Duration::from_secs(120);
            loop {
                if self.control.cancelled.load(Ordering::SeqCst) {
                    return Err("MCP check cancelled".into());
                }
                match rx.recv_timeout(Duration::from_millis(250)) {
                    Ok(decision) => {
                        (self.validate)()?;
                        return Ok(decision);
                    }
                    Err(mpsc::RecvTimeoutError::Disconnected) => {
                        return Err("MCP approval closed".into())
                    }
                    Err(mpsc::RecvTimeoutError::Timeout) => {
                        if Instant::now() >= deadline {
                            return Err("MCP approval timed out".into());
                        }
                        self.channel
                            .send(InspectionEvent::Waiting)
                            .map_err(|_| "MCP check view closed")?;
                    }
                }
            }
        })();
        self.control
            .approval
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take();
        result
    }
}
#[tauri::command]
pub async fn inspect_mcp_server(
    request: InspectionRequest,
    channel: Channel<InspectionEvent>,
    state: State<'_, DesktopState>,
) -> Result<InspectionResult, String> {
    let pending = PendingInspection::reserve(&request.id)?;
    let state = state.inner().clone();
    crate::commands::run_blocking("desktop.inspect_mcp_server", move || {
        let store = McpConfigStore::from_environment()?;
        let snapshot = store.load()?;
        if snapshot.fingerprint != request.fingerprint {
            return Err("MCP configuration changed. Refresh connections and check again.".into());
        }
        let context = &request.context;
        let (workspace, mode) = state.mcp_inspection_context(
            context.conversation_id.as_ref(),
            context.workspace_path.as_deref(),
            context.permission_mode,
        )?;
        let validate = || {
            if pending.control.cancelled.load(Ordering::SeqCst) {
                return Err("MCP check cancelled".into());
            }
            if store.load()?.fingerprint != request.fingerprint {
                return Err(
                    "MCP configuration changed. Refresh connections and check again.".into(),
                );
            }
            if state.mcp_inspection_context(
                context.conversation_id.as_ref(),
                context.workspace_path.as_deref(),
                context.permission_mode,
            )? != (workspace.clone(), mode)
            {
                return Err("Chat workspace or permissions changed. Check again.".into());
            }
            Ok(())
        };
        let authorizer = SettingsAuthorizer {
            control: &pending.control,
            channel: &channel,
            validate: &validate,
        };
        validate()?;
        let tools = inspect_server_tools(
            snapshot.config,
            &request.server,
            &workspace,
            mode,
            &authorizer,
            &pending.control.cancelled,
        )?;
        validate()?;
        Ok(InspectionResult {
            tools,
            workspace_path: workspace.display().to_string(),
            fingerprint: request.fingerprint,
        })
    })
    .await
}
#[tauri::command]
pub fn resolve_mcp_inspection_approval(
    id: String,
    approval_id: String,
    allow: bool,
) -> Result<(), String> {
    resolve(&id, &approval_id, allow)
}
#[tauri::command]
pub fn cancel_mcp_inspection(id: String) -> Result<(), String> {
    cancel(&id)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cancellation_arriving_before_the_worker_starts_is_not_lost() {
        cancel("test-early-cancel").unwrap();
        assert!(PendingInspection::reserve("test-early-cancel").is_err());
    }
    #[test]
    fn approvals_are_operation_owned_single_use_and_cancellation_releases_waiters() {
        let first = PendingInspection::reserve("test-first").unwrap();
        let second = PendingInspection::reserve("test-second").unwrap();
        assert!(PendingInspection::reserve("test-first").is_err());
        let (tx, rx) = std::sync::mpsc::channel();
        *first.control.approval.lock().unwrap() = Some(("approval".into(), tx));
        assert!(resolve("test-second", "approval", true).is_err());
        assert!(resolve("test-first", "wrong", true).is_err());
        resolve("test-first", "approval", false).unwrap();
        assert_eq!(rx.recv().unwrap(), AuthorizationDecision::Deny);
        assert!(resolve("test-first", "approval", true).is_err());
        let (tx, rx) = std::sync::mpsc::channel();
        *first.control.approval.lock().unwrap() = Some(("next".into(), tx));
        cancel("test-first").unwrap();
        assert_eq!(rx.recv().unwrap(), AuthorizationDecision::Deny);
        assert!(first.control.cancelled.load(Ordering::SeqCst));
        assert!(!second.control.cancelled.load(Ordering::SeqCst));
        drop(first);
        assert!(PendingInspection::reserve("test-first").is_ok());
    }
}
