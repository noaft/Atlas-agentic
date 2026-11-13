//! Backend-owned process resources; scopes are capabilities, never model-selected IDs.
use super::{
    output::{Completion, ProcessControl},
    ExecArgs, ProcessCursor, ProcessLifetime, ProcessRead, OUTPUT_LIMIT,
};
use crate::{
    process_limits::ProcessLimits,
    tools::{ExecCommandTool, Tool, ToolResult},
};
use serde::Serialize;
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};
const PER_SCOPE_RECORDS: usize = ProcessLimits::MAX_PER_CHAT;
const GLOBAL_RECORDS: usize = ProcessLimits::MAX_TOTAL;
struct Session {
    owner: String,
    request: String,
    lifetime: ProcessLifetime,
    workspace: PathBuf,
    control: Arc<ProcessControl>,
    worker: Option<JoinHandle<()>>,
    created: Instant,
    started_at: String,
    timeout_ms: u64,
}
#[derive(Default)]
struct Store {
    sessions: HashMap<String, Session>,
    closing: bool,
    limits: ProcessLimits,
    synchronous: HashMap<String, String>,
}
fn check_admission(store: &Store, owner: &str) -> Result<(), String> {
    if store.closing {
        return Err("Process manager is shutting down".into());
    }
    let active = store
        .sessions
        .values()
        .filter(|s| !s.control.is_done())
        .count()
        + store.synchronous.len();
    if active >= store.limits.total {
        return Err(format!(
            "App limit of {} concurrent commands reached; wait for or stop an existing command",
            store.limits.total
        ));
    }
    let chat_active = store
        .sessions
        .values()
        .filter(|s| s.owner == owner && !s.control.is_done())
        .count()
        + store
            .synchronous
            .values()
            .filter(|scope| scope.as_str() == owner)
            .count();
    if chat_active >= store.limits.per_chat {
        return Err(format!(
            "Chat limit of {} concurrent commands reached; wait for or stop an existing command",
            store.limits.per_chat
        ));
    }
    Ok(())
}

pub(crate) struct SynchronousAdmission {
    inner: Arc<Inner>,
    id: String,
}
impl Drop for SynchronousAdmission {
    fn drop(&mut self) {
        self.inner
            .store
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .synchronous
            .remove(&self.id);
    }
}

#[derive(Default)]
struct Inner {
    store: Mutex<Store>,
}
#[derive(Clone, Default)]
pub struct ProcessManager {
    inner: Arc<Inner>,
}
#[derive(Clone)]
pub struct ProcessScope {
    inner: Arc<Inner>,
    id: String,
    closed: Arc<AtomicBool>,
}
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProcessInfo {
    pub session_id: String,
    pub kind: &'static str,
    pub lifetime: ProcessLifetime,
    pub status: &'static str,
    pub running: bool,
    pub workspace_path: PathBuf,
    pub started_at: String,
    pub timeout_ms: u64,
    pub exit_code: Option<i32>,
}
impl ProcessManager {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn set_limits(&self, limits: ProcessLimits) -> Result<(), String> {
        self.inner.set_limits(limits)
    }
    pub fn create_scope(&self) -> Result<ProcessScope, String> {
        if self
            .inner
            .store
            .lock()
            .map_err(|_| "Process registry unavailable")?
            .closing
        {
            return Err("Process manager is shutting down".into());
        }
        Ok(ProcessScope {
            inner: self.inner.clone(),
            id: uuid::Uuid::new_v4().to_string(),
            closed: Arc::new(AtomicBool::new(false)),
        })
    }
    pub fn shutdown(&self, timeout: Duration) -> Result<(), String> {
        let ids = {
            let mut store = self.inner.store.lock().unwrap_or_else(|e| e.into_inner());
            store.closing = true;
            store.sessions.keys().cloned().collect::<Vec<_>>()
        };
        self.inner.stop_and_join(&ids, timeout)
    }
}
impl ProcessScope {
    pub fn id(&self) -> &str {
        &self.id
    }
    fn check(&self) -> Result<(), String> {
        if self.closed.load(Ordering::SeqCst) {
            Err("Process scope is closed".into())
        } else {
            Ok(())
        }
    }
    pub fn list(&self) -> Result<Vec<ProcessInfo>, String> {
        self.check()?;
        let store = self
            .inner
            .store
            .lock()
            .map_err(|_| "Process registry unavailable")?;
        let mut items = store
            .sessions
            .iter()
            .filter(|(_, s)| s.owner == self.id)
            .map(|(id, s)| {
                let (status, running, exit_code) = s.control.state();
                (
                    s.created,
                    ProcessInfo {
                        session_id: id.clone(),
                        kind: "process",
                        lifetime: s.lifetime,
                        status,
                        running,
                        workspace_path: s.workspace.clone(),
                        started_at: s.started_at.clone(),
                        timeout_ms: s.timeout_ms,
                        exit_code,
                    },
                )
            })
            .collect::<Vec<_>>();
        items.sort_by_key(|(created, _)| *created);
        Ok(items.into_iter().map(|(_, info)| info).collect())
    }
    fn control(&self, id: &str) -> Result<Arc<ProcessControl>, String> {
        self.check()?;
        self.inner
            .store
            .lock()
            .map_err(|_| "Process registry unavailable")?
            .sessions
            .get(id)
            .filter(|s| s.owner == self.id)
            .map(|s| s.control.clone())
            .ok_or("Unknown process session in this chat".into())
    }
    pub fn read(
        &self,
        id: &str,
        cursor: Option<ProcessCursor>,
        wait_ms: u64,
    ) -> Result<ProcessRead, String> {
        if wait_ms > 10_000 {
            return Err("wait_ms must be 0–10000".into());
        }
        let c = self.control(id)?;
        if let Some(cursor) = cursor {
            c.snapshot(id, Some(cursor), Duration::from_millis(wait_ms), false)
        } else {
            c.read_agent(id, Duration::from_millis(wait_ms), false)
        }
    }
    /// Inspect the latest retained lines without advancing any reader's cursor.
    pub fn read_tail(&self, id: &str, lines: usize, wait_ms: u64) -> Result<ProcessRead, String> {
        if !(1..=1000).contains(&lines) || wait_ms > 10_000 {
            return Err("tail_lines must be 1–1000; wait_ms must be 0–10000".into());
        }
        self.control(id)?
            .tail_snapshot(id, lines, Duration::from_millis(wait_ms))
    }
    pub fn stop(&self, id: &str) -> Result<ProcessRead, String> {
        let c = self.control(id)?;
        if !c.is_done() {
            c.cancel();
        }
        c.snapshot(
            id,
            Some(ProcessCursor::default()),
            Duration::from_secs(2),
            true,
        )
    }
    fn stop_agent(&self, id: &str) -> Result<ProcessRead, String> {
        let c = self.control(id)?;
        if !c.is_done() {
            c.cancel();
        }
        c.read_agent(id, Duration::from_secs(2), true)
    }
    /// Native ownership handoff, preserving resource IDs. Never exposed as a model tool.
    pub fn adopt_from(&self, source: &ProcessScope) -> Result<(), String> {
        if !Arc::ptr_eq(&self.inner, &source.inner) {
            return Err("Cannot transfer between process managers".into());
        }
        if self.id == source.id {
            return Ok(());
        }
        let mut store = self
            .inner
            .store
            .lock()
            .map_err(|_| "Process registry unavailable")?;
        self.check()?;
        source.check()?;
        let target_active = store
            .sessions
            .values()
            .filter(|s| s.owner == self.id && !s.control.is_done())
            .count();
        let source_active = store
            .sessions
            .values()
            .filter(|s| s.owner == source.id && !s.control.is_done())
            .count();
        // Renaming an existing chat's owner must not terminate grandfathered work.
        // Combining two active owners still has to fit the current per-chat cap.
        if target_active > 0
            && source_active > 0
            && target_active + source_active > store.limits.per_chat
        {
            return Err("Process ownership transfer exceeds the chat active limit".into());
        }
        while store
            .sessions
            .values()
            .filter(|s| s.owner == self.id || s.owner == source.id)
            .count()
            > PER_SCOPE_RECORDS
        {
            let oldest = store
                .sessions
                .iter()
                .filter(|(_, s)| {
                    (s.owner == self.id || s.owner == source.id) && s.control.is_done()
                })
                .min_by_key(|(_, s)| s.created)
                .map(|(id, _)| id.clone())
                .ok_or("Process transfer has too many records")?;
            store.sessions.remove(&oldest);
        }
        for session in store.sessions.values_mut().filter(|s| s.owner == source.id) {
            session.owner = self.id.clone();
        }
        source.closed.store(true, Ordering::SeqCst);
        Ok(())
    }
    pub fn close(&self, timeout: Duration) -> Result<(), String> {
        let ids = {
            let store = self.inner.store.lock().unwrap_or_else(|e| e.into_inner());
            self.closed.store(true, Ordering::SeqCst);
            store
                .sessions
                .iter()
                .filter(|(_, s)| s.owner == self.id)
                .map(|(id, _)| id.clone())
                .collect::<Vec<_>>()
        };
        let result = self.inner.stop_and_join(&ids, timeout);
        self.inner
            .store
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .sessions
            .retain(|_, s| s.owner != self.id || !s.control.is_done());
        result
    }
}
impl Inner {
    fn set_limits(&self, limits: ProcessLimits) -> Result<(), String> {
        limits.validate()?;
        self.store
            .lock()
            .map_err(|_| "Process registry unavailable")?
            .limits = limits;
        Ok(())
    }
    fn stop_and_join(&self, ids: &[String], timeout: Duration) -> Result<(), String> {
        {
            let store = self.store.lock().unwrap_or_else(|e| e.into_inner());
            for id in ids {
                if let Some(s) = store.sessions.get(id) {
                    if !s.control.is_done() {
                        s.control.cancel();
                    }
                }
            }
        }
        let deadline = Instant::now() + timeout;
        loop {
            let pending = {
                let mut store = self.store.lock().unwrap_or_else(|e| e.into_inner());
                let mut pending = false;
                for id in ids {
                    if let Some(s) = store.sessions.get_mut(id) {
                        if s.worker.as_ref().is_some_and(|w| w.is_finished()) {
                            if let Some(worker) = s.worker.take() {
                                let _ = worker.join();
                            }
                        }
                        pending |= !s.control.is_done() || s.worker.is_some();
                    }
                }
                pending
            };
            if !pending {
                return Ok(());
            }
            if Instant::now() >= deadline {
                return Err(
                    "Process cleanup is still pending; action outcome is not yet confirmed".into(),
                );
            }
            thread::sleep(Duration::from_millis(10));
        }
    }
}
impl Drop for Inner {
    fn drop(&mut self) {
        let store = self.store.get_mut().unwrap_or_else(|e| e.into_inner());
        for s in store.sessions.values() {
            s.control.cancel();
        }
        let deadline = Instant::now() + Duration::from_secs(3);
        for (_, mut s) in store.sessions.drain() {
            if let Some(worker) = s.worker.take() {
                while !worker.is_finished() && Instant::now() < deadline {
                    thread::sleep(Duration::from_millis(5));
                }
                if worker.is_finished() {
                    let _ = worker.join();
                }
            }
        }
    }
}
pub(crate) struct ProcessSessions {
    scope: ProcessScope,
    request: String,
    workspace: PathBuf,
    output_limit: usize,
    persistent: bool,
    shared: bool,
    managed: bool,
}
impl ProcessSessions {
    pub(crate) fn new(workspace: &Path, limit: usize) -> Self {
        let manager = ProcessManager::new();
        Self {
            scope: manager.create_scope().expect("new manager is open"),
            request: uuid::Uuid::new_v4().to_string(),
            workspace: workspace.into(),
            output_limit: limit,
            persistent: false,
            shared: false,
            managed: false,
        }
    }
    pub(crate) fn for_scope(workspace: &Path, scope: ProcessScope) -> Self {
        Self {
            scope,
            request: uuid::Uuid::new_v4().to_string(),
            workspace: workspace.into(),
            output_limit: OUTPUT_LIMIT,
            persistent: true,
            shared: true,
            managed: true,
        }
    }
    pub(crate) fn for_request_scope(workspace: &Path, scope: ProcessScope, managed: bool) -> Self {
        let mut sessions = Self::for_scope(workspace, scope);
        sessions.persistent = false;
        sessions.managed = managed;
        sessions
    }
    pub(crate) fn admit_synchronous(&self) -> Result<SynchronousAdmission, String> {
        let mut store = self
            .scope
            .inner
            .store
            .lock()
            .map_err(|_| "Process registry unavailable")?;
        self.scope.check()?;
        check_admission(&store, &self.scope.id)?;
        let id = uuid::Uuid::new_v4().to_string();
        store.synchronous.insert(id.clone(), self.scope.id.clone());
        Ok(SynchronousAdmission {
            inner: self.scope.inner.clone(),
            id,
        })
    }
    pub(crate) fn set_limits(&self, limits: ProcessLimits) -> Result<(), String> {
        self.scope.inner.set_limits(limits)
    }
    pub(crate) fn list(&self) -> Result<Vec<ProcessInfo>, String> {
        self.scope.list()
    }
    pub(crate) fn read(
        &self,
        id: &str,
        cursor: Option<ProcessCursor>,
        wait: u64,
    ) -> Result<ProcessRead, String> {
        self.scope.read(id, cursor, wait)
    }
    pub(crate) fn read_tail(
        &self,
        id: &str,
        lines: usize,
        wait_ms: u64,
    ) -> Result<ProcessRead, String> {
        self.scope.read_tail(id, lines, wait_ms)
    }
    pub(crate) fn stop_independent(&self, id: &str) -> Result<ProcessRead, String> {
        self.scope.stop(id)
    }
    pub(crate) fn stop(&self, id: &str) -> Result<ProcessRead, String> {
        self.scope.stop_agent(id)
    }
    pub(crate) fn validate(&self, id: &str) -> Result<(), String> {
        self.scope.control(id).map(|_| ())
    }
    pub(crate) fn is_scoped(&self) -> bool {
        self.managed
    }
    pub(crate) fn start(&self, args: ExecArgs) -> Result<ToolResult, String> {
        if args.lifetime == ProcessLifetime::Background && !self.persistent {
            return Err(
                "Background processes require a desktop chat scope; CLI sessions are request-local"
                    .into(),
            );
        }
        self.scope.check()?;
        let synchronous = args.yield_time_ms.is_none();
        let timeout_ms =
            args.timeout_ms
                .unwrap_or(if args.lifetime == ProcessLifetime::Background {
                    3_600_000
                } else if synchronous {
                    30_000
                } else {
                    600_000
                });
        let wait = Duration::from_millis(
            args.yield_time_ms
                .unwrap_or(timeout_ms.saturating_add(2000)),
        );
        let command = synchronous.then(|| args.command.clone());
        let timeout = Duration::from_millis(timeout_ms);
        let id = uuid::Uuid::new_v4().to_string();
        let control = Arc::new(ProcessControl::new(self.output_limit));
        {
            let mut store = self
                .scope
                .inner
                .store
                .lock()
                .map_err(|_| "Process registry unavailable")?;
            self.scope.check()?;
            check_admission(&store, &self.scope.id)?;
            let scope_count = store
                .sessions
                .values()
                .filter(|s| s.owner == self.scope.id)
                .count();
            if scope_count >= PER_SCOPE_RECORDS || store.sessions.len() >= GLOBAL_RECORDS {
                let same_scope = scope_count >= PER_SCOPE_RECORDS;
                let oldest = store
                    .sessions
                    .iter()
                    .filter(|(_, s)| {
                        s.control.is_done() && (!same_scope || s.owner == self.scope.id)
                    })
                    .min_by_key(|(_, s)| s.created)
                    .map(|(id, _)| id.clone());
                if let Some(oldest) = oldest {
                    store.sessions.remove(&oldest);
                } else {
                    return Err("Process record limit reached".into());
                }
            }
            let worker_control = control.clone();
            let workspace = self.workspace.clone();
            let limit = self.output_limit;
            let worker = thread::Builder::new()
                .name("atlas-process".into())
                .spawn(move || {
                    let completion = if worker_control.stopped() {
                        Completion {
                            stopped: true,
                            ..Completion::default()
                        }
                    } else {
                        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                            ExecCommandTool::with_limits(workspace, timeout, limit)
                                .with_control(worker_control.clone())
                                .execute(&serde_json::json!({"command":args.command}))
                        }));
                        match result {
                            Ok(Ok(result)) => serde_json::from_str::<Completion>(&result.message)
                                .unwrap_or_else(|_| Completion {
                                    error: Some("Invalid process completion".into()),
                                    ..Completion::default()
                                }),
                            Ok(Err(error)) => Completion {
                                error: Some(error),
                                ..Completion::default()
                            },
                            Err(_) => Completion {
                                error: Some("Process worker failed".into()),
                                ..Completion::default()
                            },
                        }
                    };
                    worker_control.complete(completion);
                })
                .map_err(|_| "Could not start process session worker")?;
            store.sessions.insert(
                id.clone(),
                Session {
                    owner: self.scope.id.clone(),
                    request: self.request.clone(),
                    lifetime: args.lifetime,
                    workspace: self.workspace.clone(),
                    control: control.clone(),
                    worker: Some(worker),
                    created: Instant::now(),
                    started_at: chrono::Utc::now().to_rfc3339(),
                    timeout_ms,
                },
            );
        }
        let read = control.read_agent(&id, wait, true)?;
        let mut value =
            serde_json::to_value(read).map_err(|_| "Could not encode process result")?;
        if let Some(command) = command {
            value["command"] = command.into();
        }
        Ok(ToolResult::text(value.to_string()))
    }
    pub(crate) fn clear(&self) {
        let ids = {
            let store = self
                .scope
                .inner
                .store
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            store
                .sessions
                .iter()
                .filter(|(_, s)| {
                    s.owner == self.scope.id
                        && ((!self.persistent && !self.shared)
                            || (s.request == self.request && s.lifetime == ProcessLifetime::Job))
                })
                .map(|(id, _)| id.clone())
                .collect::<Vec<_>>()
        };
        let _ = self.scope.inner.stop_and_join(&ids, Duration::from_secs(3));
        if !self.persistent {
            self.scope
                .inner
                .store
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .sessions
                .retain(|_, s| {
                    s.owner != self.scope.id
                        || !s.control.is_done()
                        || (self.shared && s.request != self.request)
                });
        }
    }
}
impl Drop for ProcessSessions {
    fn drop(&mut self) {
        self.clear();
    }
}

#[cfg(all(test, unix))]
mod delegated_scope_tests {
    use super::*;
    #[test]
    fn cli_worker_synchronous_commands_are_contained_and_stopped_on_scope_exit() {
        let folder = tempfile::tempdir().unwrap();
        let manager = ProcessManager::new();
        let scope = manager.create_scope().unwrap();
        let workspace = folder.path().to_path_buf();
        let owned = scope.clone();
        let worker = std::thread::spawn(move || {
            crate::tools::ExecCommandTool::new(workspace)
                .with_request_scope(owned, true)
                .execute(&serde_json::json!({"command":"sleep 30","timeout_ms":60000}))
        });
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            if scope.list().unwrap().iter().any(|session| session.running) {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "worker command must be tracked by its shared scope"
            );
            std::thread::sleep(Duration::from_millis(5));
        }
        scope.close(Duration::from_secs(3)).unwrap();
        let output = worker.join().unwrap().unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&output.message).unwrap();
        assert_eq!(parsed["running"], false);
    }
    #[test]
    fn request_workers_share_chat_command_caps_and_preserve_other_requests_jobs() {
        let folder = tempfile::tempdir().unwrap();
        let manager = ProcessManager::new();
        manager
            .set_limits(crate::process_limits::ProcessLimits {
                per_chat: 1,
                total: 1,
            })
            .unwrap();
        let scope = manager.create_scope().unwrap();
        let root = ProcessSessions::for_scope(folder.path(), scope.clone());
        let child = ProcessSessions::for_request_scope(folder.path(), scope.clone(), true);
        root.start(ExecArgs {
            command: "sleep 5".into(),
            yield_time_ms: Some(0),
            timeout_ms: Some(10000),
            lifetime: ProcessLifetime::Job,
        })
        .unwrap();
        assert!(child.admit_synchronous().is_err());
        child.clear();
        assert!(scope.list().unwrap().iter().any(|process| process.running));
        assert!(child
            .start(ExecArgs {
                command: "true".into(),
                yield_time_ms: Some(0),
                timeout_ms: Some(1000),
                lifetime: ProcessLifetime::Background
            })
            .is_err());
        root.clear();
    }
}
