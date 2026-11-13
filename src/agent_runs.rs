//! Owner-scoped, bounded cooperative background agent execution.
use crate::models::ModelUsage;
use crate::permissions::{AuthorizationDecision, AuthorizationRequest};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{HashSet, VecDeque};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::{Duration, Instant};

const MAX_RESULT_BYTES: usize = 64 * 1024;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ContextMode {
    Fork,
    TaskOnly,
}
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionMode {
    Wait,
    Parallel,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentRunRequest {
    pub agent: String,
    pub task: String,
    pub tools: Vec<String>,
    pub context_mode: ContextMode,
    pub execution_mode: ExecutionMode,
}
impl AgentRunRequest {
    pub fn validate(&self) -> Result<(), String> {
        if !identifier(&self.agent) || self.task.trim().is_empty() || self.task.len() > 65536 {
            return Err("invalid agent run agent or task".into());
        }
        let mut seen = HashSet::new();
        if self.tools.len() > 64
            || self
                .tools
                .iter()
                .any(|tool| !identifier(tool) || !seen.insert(tool))
        {
            return Err("invalid or duplicate agent run tools".into());
        }
        Ok(())
    }
}
fn identifier(value: &str) -> bool {
    !value.trim().is_empty() && value.chars().count() <= 128 && !value.chars().any(char::is_control)
}
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AgentOutcome {
    Done,
    Partial,
    Blocked,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentResult {
    pub outcome: AgentOutcome,
    pub summary: String,
    pub work_done: Vec<String>,
    pub findings: Vec<Value>,
    pub artifacts: Vec<String>,
    pub verification: Vec<Value>,
    pub remaining: Vec<String>,
}
impl AgentResult {
    pub fn parse(text: &str) -> Result<Self, String> {
        if text.len() > MAX_RESULT_BYTES {
            return Err("agent result exceeds 64 KiB".into());
        }
        let result: Self = serde_json::from_str(text)
            .map_err(|_| "invalid structured agent result".to_string())?;
        result.validate()?;
        Ok(result)
    }
    pub fn validate(&self) -> Result<(), String> {
        if self.summary.trim().is_empty()
            || self.summary.len() > 8192
            || [
                self.work_done.len(),
                self.findings.len(),
                self.artifacts.len(),
                self.verification.len(),
                self.remaining.len(),
            ]
            .iter()
            .any(|n| *n > 64)
        {
            return Err("invalid or oversized agent result".into());
        }
        let bytes =
            serde_json::to_vec(self).map_err(|_| "invalid structured agent result".to_string())?;
        if bytes.len() > MAX_RESULT_BYTES {
            return Err("agent result exceeds 64 KiB".into());
        }
        let value: Value = serde_json::from_slice(&bytes)
            .map_err(|_| "invalid structured agent result".to_string())?;
        if contains_image_payload(&value) {
            return Err("agent result cannot contain image payloads".into());
        }
        Ok(())
    }
}
fn contains_image_payload(value: &Value) -> bool {
    match value {
        Value::String(text) => text.to_ascii_lowercase().contains("data:image/"),
        Value::Array(values) => values.iter().any(contains_image_payload),
        Value::Object(values) => values.iter().any(|(key, value)| {
            matches!(key.as_str(), "image_data" | "image_base64" | "b64_json")
                || contains_image_payload(value)
        }),
        _ => false,
    }
}
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AgentRunStatus {
    Queued,
    Running,
    WaitingApproval,
    Cancelling,
    Completed,
    Failed,
    Cancelled,
    TimedOut,
}
impl AgentRunStatus {
    pub fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::Completed | Self::Failed | Self::Cancelled | Self::TimedOut
        )
    }
}
#[derive(Debug, Clone, Serialize)]
pub struct RunActivity {
    pub tool: String,
    pub success: Option<bool>,
    pub summary: String,
}
#[derive(Debug, Clone, Serialize)]
pub struct AgentRunView {
    pub run_id: String,
    pub parent_run_id: Option<String>,
    pub agent: String,
    pub task: String,
    pub context_mode: ContextMode,
    pub execution_mode: ExecutionMode,
    pub status: AgentRunStatus,
    pub tools: Vec<String>,
    pub model: Option<String>,
    pub usage: Option<ModelUsage>,
    pub progress: Option<String>,
    pub current_tool: Option<String>,
    pub started_at: String,
    pub duration_ms: u64,
    pub tool_calls: u64,
    pub pending_approval: Option<AuthorizationRequest>,
    pub activity: Vec<RunActivity>,
    pub result: Option<AgentResult>,
    pub error: Option<String>,
}
struct Record {
    owner: String,
    ancestor_run_ids: Vec<String>,
    view: AgentRunView,
    started: Instant,
    deadline: Instant,
    cancelled: bool,
    approval_decision: Option<AuthorizationDecision>,
}
impl Record {
    fn snapshot(&self) -> AgentRunView {
        let mut view = self.view.clone();
        if !view.status.is_terminal() {
            view.duration_ms = elapsed_ms(self.started);
        }
        view
    }
}
#[derive(Default)]
struct Store {
    records: VecDeque<Record>,
    closed: HashSet<String>,
    shutdown: bool,
}
#[derive(Default)]
struct Shared {
    store: Mutex<Store>,
    changed: Condvar,
}
#[derive(Clone, Default)]
pub struct AgentRunManager {
    shared: Arc<Shared>,
}
#[derive(Clone)]
pub struct RunControl {
    manager: AgentRunManager,
    run_id: String,
}
impl AgentRunManager {
    fn lock(&self) -> Result<MutexGuard<'_, Store>, String> {
        self.shared
            .store
            .lock()
            .map_err(|_| "agent run manager unavailable".into())
    }
    pub fn start<F>(
        &self,
        owner: &str,
        request: AgentRunRequest,
        timeout: Duration,
        worker: F,
    ) -> Result<AgentRunView, String>
    where
        F: FnOnce(RunControl) -> Result<AgentResult, String> + Send + 'static,
    {
        self.start_with_parent(owner, request, timeout, None, worker)
    }
    pub fn start_with_parent<F>(
        &self,
        owner: &str,
        request: AgentRunRequest,
        timeout: Duration,
        parent_run_id: Option<&str>,
        worker: F,
    ) -> Result<AgentRunView, String>
    where
        F: FnOnce(RunControl) -> Result<AgentResult, String> + Send + 'static,
    {
        request.validate()?;
        let started = Instant::now();
        let deadline = started
            .checked_add(timeout)
            .filter(|_| !timeout.is_zero())
            .ok_or("invalid agent run timeout")?;
        let mut store = self.lock()?;
        if owner.is_empty() || store.shutdown || store.closed.contains(owner) {
            return Err("agent run owner is closed".into());
        }
        let ancestor_run_ids = if let Some(parent_id) = parent_run_id {
            let parent = store
                .records
                .iter()
                .find(|record| record.owner == owner && record.view.run_id == parent_id)
                .ok_or("agent run parent not found")?;
            if parent.cancelled
                || parent.view.status.is_terminal()
                || Instant::now() >= parent.deadline
            {
                return Err("agent run parent is no longer active".into());
            }
            if parent.ancestor_run_ids.len() >= 64 {
                return Err("agent run ancestry limit reached".into());
            }
            let mut ancestors = parent.ancestor_run_ids.clone();
            ancestors.push(parent_id.to_string());
            ancestors
        } else {
            Vec::new()
        };
        let active = |record: &&Record| !record.view.status.is_terminal();
        if store.records.iter().filter(active).count() >= 16
            || store
                .records
                .iter()
                .filter(|record| record.owner == owner && !record.view.status.is_terminal())
                .count()
                >= 4
        {
            return Err("agent run concurrency limit reached".into());
        }
        trim_retained(&mut store, owner);
        let view = AgentRunView {
            run_id: uuid::Uuid::new_v4().to_string(),
            parent_run_id: parent_run_id.map(str::to_string),
            agent: request.agent,
            task: request.task,
            tools: request.tools,
            context_mode: request.context_mode,
            execution_mode: request.execution_mode,
            status: AgentRunStatus::Running,
            model: None,
            usage: None,
            progress: None,
            current_tool: None,
            started_at: chrono::Utc::now().to_rfc3339(),
            duration_ms: 0,
            tool_calls: 0,
            pending_approval: None,
            activity: Vec::new(),
            result: None,
            error: None,
        };
        store.records.push_back(Record {
            owner: owner.into(),
            ancestor_run_ids,
            view: view.clone(),
            started,
            deadline,
            cancelled: false,
            approval_decision: None,
        });
        let control = RunControl {
            manager: self.clone(),
            run_id: view.run_id.clone(),
        };
        let run_id = view.run_id.clone();
        let manager = self.clone();
        // Admission remains locked until spawn succeeds, so no orphaned record is visible.
        if std::thread::Builder::new()
            .name("atlas-agent-run".into())
            .spawn(move || {
                let outcome = if let Err(error) = control.check() {
                    Ok(Err(error))
                } else {
                    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        let mut result = Err("agent worker did not return".to_string());
                        // Keep provider/model events correlated on this worker without logging
                        // arbitrary model/tool-derived error bodies or the assigned task.
                        let _ = crate::diagnostics::run(
                            "agent.worker",
                            serde_json::json!({"runId":run_id}),
                            || {
                                result = worker(control);
                                result
                                    .as_ref()
                                    .map(|_| ())
                                    .map_err(|_| "agent worker failed".to_string())
                            },
                        );
                        result
                    }))
                };
                manager.finish(&run_id, outcome);
            })
            .is_err()
        {
            store
                .records
                .retain(|record| record.view.run_id != view.run_id);
            return Err("unable to start agent worker".into());
        }
        crate::diagnostics::event(
            "INFO",
            "agent.started",
            serde_json::json!({"runId":view.run_id,"parentRunId":view.parent_run_id}),
        );
        Ok(view)
    }
    fn finish(&self, id: &str, outcome: std::thread::Result<Result<AgentResult, String>>) {
        let Ok(mut store) = self.lock() else {
            return;
        };
        let Some(record) = store
            .records
            .iter_mut()
            .find(|record| record.view.run_id == id)
        else {
            return;
        };
        let view = &mut record.view;
        view.duration_ms = elapsed_ms(record.started);
        view.current_tool = None;
        view.pending_approval = None;
        record.approval_decision = None;
        if Instant::now() >= record.deadline {
            view.status = AgentRunStatus::TimedOut;
        } else if record.cancelled {
            view.status = AgentRunStatus::Cancelled;
        } else {
            match outcome {
                Ok(Ok(result)) => match result.validate() {
                    Ok(()) => {
                        view.status = AgentRunStatus::Completed;
                        view.result = Some(result);
                    }
                    Err(error) => {
                        view.status = AgentRunStatus::Failed;
                        view.error = Some(error);
                    }
                },
                Ok(Err(error)) => {
                    view.status = AgentRunStatus::Failed;
                    view.error = Some(bounded(&error, 4096));
                }
                Err(_) => {
                    view.status = AgentRunStatus::Failed;
                    view.error = Some("agent worker panicked".into());
                }
            }
        }
        if view.status != AgentRunStatus::Completed {
            cancel_descendants(&mut store, id);
        }
        self.shared.changed.notify_all();
    }
    /// Resolve exactly one pending approval using native owner authority.
    pub fn resolve_approval(
        &self,
        owner: &str,
        id: &str,
        approval_id: &str,
        allow: bool,
    ) -> Result<(), String> {
        let mut store = self.lock()?;
        let record = store
            .records
            .iter_mut()
            .find(|record| record.owner == owner && record.view.run_id == id)
            .ok_or("agent run not found")?;
        if record.cancelled
            || record.view.status.is_terminal()
            || Instant::now() >= record.deadline
            || record
                .view
                .pending_approval
                .as_ref()
                .map(|request| request.id.as_str())
                != Some(approval_id)
        {
            return Err("agent approval is no longer pending".into());
        }
        record.view.pending_approval = None;
        record.approval_decision = Some(if allow {
            AuthorizationDecision::Allow
        } else {
            AuthorizationDecision::Deny
        });
        self.shared.changed.notify_all();
        Ok(())
    }
    pub fn list(&self, owner: &str) -> Result<Vec<AgentRunView>, String> {
        Ok(self
            .lock()?
            .records
            .iter()
            .filter(|record| record.owner == owner)
            .map(Record::snapshot)
            .collect())
    }
    pub fn read(&self, owner: &str, id: &str) -> Result<AgentRunView, String> {
        self.lock()?
            .records
            .iter()
            .find(|record| record.owner == owner && record.view.run_id == id)
            .map(Record::snapshot)
            .ok_or("agent run not found".into())
    }
    pub fn cancel(&self, owner: &str, id: &str) -> Result<AgentRunView, String> {
        let mut store = self.lock()?;
        let record = store
            .records
            .iter_mut()
            .find(|record| record.owner == owner && record.view.run_id == id)
            .ok_or("agent run not found")?;
        request_cancel(record);
        let view = record.snapshot();
        cancel_descendants(&mut store, id);
        self.shared.changed.notify_all();
        Ok(view)
    }
    pub fn wait(&self, owner: &str, id: &str, timeout: Duration) -> Result<AgentRunView, String> {
        let until = Instant::now() + timeout.min(Duration::from_secs(10));
        let mut store = self.lock()?;
        loop {
            let view = store
                .records
                .iter()
                .find(|record| record.owner == owner && record.view.run_id == id)
                .map(Record::snapshot)
                .ok_or("agent run not found")?;
            if view.status.is_terminal() || Instant::now() >= until {
                return Ok(view);
            }
            let (guard, _) = self
                .shared
                .changed
                .wait_timeout(store, until.saturating_duration_since(Instant::now()))
                .map_err(|_| "agent run manager unavailable")?;
            store = guard;
        }
    }
    /// Transfer native scope ownership without changing run identities or worker controls.
    /// A rejected merge leaves both owners and all records unchanged.
    pub fn adopt_owner(&self, from: &str, to: &str) -> Result<(), String> {
        let mut store = self.lock()?;
        if from.is_empty()
            || to.is_empty()
            || store.shutdown
            || store.closed.contains(from)
            || store.closed.contains(to)
        {
            return Err("agent run owner is closed".into());
        }
        if from == to {
            return Ok(());
        }
        let active = store
            .records
            .iter()
            .filter(|record| {
                (record.owner == from || record.owner == to) && !record.view.status.is_terminal()
            })
            .count();
        if active > 4 {
            return Err("agent run ownership merge exceeds concurrency limit".into());
        }
        for record in &mut store.records {
            if record.owner == from {
                record.owner = to.to_string();
            }
        }
        store.closed.insert(from.to_string());
        while store
            .records
            .iter()
            .filter(|record| record.owner == to)
            .count()
            > 32
        {
            let Some(index) = store
                .records
                .iter()
                .position(|record| record.owner == to && record.view.status.is_terminal())
            else {
                break;
            };
            store.records.remove(index);
        }
        self.shared.changed.notify_all();
        Ok(())
    }
    pub fn close_owner(&self, owner: &str) {
        if let Ok(mut store) = self.lock() {
            store.closed.insert(owner.into());
            for record in store
                .records
                .iter_mut()
                .filter(|record| record.owner == owner)
            {
                request_cancel(record);
            }
            self.shared.changed.notify_all();
        }
    }
    pub fn shutdown(&self) {
        if let Ok(mut store) = self.lock() {
            store.shutdown = true;
            for record in &mut store.records {
                request_cancel(record);
            }
            self.shared.changed.notify_all();
        }
    }
}
fn cancel_descendants(store: &mut Store, parent_id: &str) {
    for record in &mut store.records {
        if record
            .ancestor_run_ids
            .iter()
            .any(|ancestor| ancestor == parent_id)
        {
            request_cancel(record);
        }
    }
}
fn request_cancel(record: &mut Record) {
    if !record.view.status.is_terminal() {
        record.cancelled = true;
        record.view.pending_approval = None;
        record.approval_decision = None;
        record.view.status = AgentRunStatus::Cancelling;
    }
}
fn trim_retained(store: &mut Store, owner: &str) {
    while store
        .records
        .iter()
        .filter(|record| record.owner == owner)
        .count()
        >= 32
    {
        let Some(index) = store
            .records
            .iter()
            .position(|record| record.owner == owner && record.view.status.is_terminal())
        else {
            break;
        };
        store.records.remove(index);
    }
    while store.records.len() >= 128 {
        let Some(index) = store
            .records
            .iter()
            .position(|record| record.view.status.is_terminal())
        else {
            break;
        };
        store.records.remove(index);
    }
}
fn elapsed_ms(started: Instant) -> u64 {
    started.elapsed().as_millis().min(u64::MAX as u128) as u64
}
fn bounded(text: &str, limit: usize) -> String {
    let mut end = text.len().min(limit);
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    text[..end].to_string()
}
impl RunControl {
    pub fn run_id(&self) -> &str {
        &self.run_id
    }
    /// Wait for an explicit run-scoped decision; cancellation never grants approval.
    pub fn authorize(
        &self,
        request: &AuthorizationRequest,
    ) -> Result<AuthorizationDecision, String> {
        self.check()?;
        if request.id.is_empty()
            || request.id.len() > 128
            || request.tool.len() > 128
            || request.arguments.len() > 8192
            || request.reason.len() > 4096
        {
            return Err("invalid agent approval request".into());
        }
        let mut store = self.manager.lock()?;
        let record = store
            .records
            .iter_mut()
            .find(|record| record.view.run_id == self.run_id)
            .ok_or("agent run not found")?;
        if record.view.pending_approval.is_some() || record.approval_decision.is_some() {
            return Err("agent approval already pending".into());
        }
        record.view.pending_approval = Some(request.clone());
        record.view.status = AgentRunStatus::WaitingApproval;
        self.manager.shared.changed.notify_all();
        loop {
            let record = store
                .records
                .iter_mut()
                .find(|record| record.view.run_id == self.run_id)
                .ok_or("agent run not found")?;
            if record.cancelled
                || record.view.status.is_terminal()
                || Instant::now() >= record.deadline
            {
                record.view.pending_approval = None;
                record.approval_decision = None;
                if !record.view.status.is_terminal() {
                    record.view.status = AgentRunStatus::Cancelling;
                }
                return Err("agent run approval cancelled or timed out".into());
            }
            if let Some(decision) = record.approval_decision.take() {
                record.view.pending_approval = None;
                record.view.status = AgentRunStatus::Running;
                return Ok(decision);
            }
            let timeout = record
                .deadline
                .saturating_duration_since(Instant::now())
                .min(Duration::from_millis(100));
            let (guard, _) = self
                .manager
                .shared
                .changed
                .wait_timeout(store, timeout)
                .map_err(|_| "agent run manager unavailable")?;
            store = guard;
        }
    }
    /// Summaries must be sanitized by the runtime; raw arguments and images do not belong here.
    pub fn record_tool(&self, tool: &str, success: Option<bool>, summary: &str) {
        self.change(|view| {
            if view.activity.len() >= 128 {
                view.activity.remove(0);
            }
            view.activity.push(RunActivity {
                tool: bounded(tool, 128),
                success,
                summary: summary.chars().take(512).collect(),
            });
        });
    }
    pub fn cancelled(&self) -> bool {
        self.check().is_err()
    }
    pub fn check(&self) -> Result<(), String> {
        let store = self.manager.lock()?;
        let record = store
            .records
            .iter()
            .find(|record| record.view.run_id == self.run_id)
            .ok_or("agent run not found")?;
        if Instant::now() >= record.deadline {
            Err("agent run timed out".into())
        } else if record.cancelled || record.view.status.is_terminal() {
            Err("agent run cancelled".into())
        } else {
            Ok(())
        }
    }
    fn change(&self, change: impl FnOnce(&mut AgentRunView)) {
        if let Ok(mut store) = self.manager.lock() {
            if let Some(record) = store
                .records
                .iter_mut()
                .find(|record| record.view.run_id == self.run_id)
            {
                if !record.view.status.is_terminal()
                    && !record.cancelled
                    && Instant::now() < record.deadline
                {
                    change(&mut record.view);
                }
            }
            self.manager.shared.changed.notify_all();
        }
    }
    pub fn set_model(&self, model: &str) {
        self.change(|view| view.model = Some(bounded(model, 128)));
    }
    /// Replace the latest model-call reading; repeated provider events are not additive.
    pub fn record_usage(&self, usage: Option<&ModelUsage>) {
        self.change(|view| view.usage = usage.cloned());
    }
    pub fn update(&self, progress: Option<&str>, current_tool: Option<&str>) {
        self.change(|view| {
            view.progress = progress.map(|text| bounded(text, 4096));
            view.current_tool = current_tool.map(|text| bounded(text, 128));
        });
    }
    pub fn increment_tool_calls(&self) {
        self.change(|view| view.tool_calls = view.tool_calls.saturating_add(1));
    }
    pub fn waiting_approval(&self, waiting: bool) {
        self.change(|view| {
            view.status = if waiting {
                AgentRunStatus::WaitingApproval
            } else {
                AgentRunStatus::Running
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;

    fn request() -> AgentRunRequest {
        AgentRunRequest {
            agent: "helper".into(),
            task: "Inspect sources".into(),
            tools: vec!["read_file".into()],
            context_mode: ContextMode::TaskOnly,
            execution_mode: ExecutionMode::Parallel,
        }
    }
    fn result() -> AgentResult {
        AgentResult::parse(r#"{"outcome":"done","summary":"Inspected sources","work_done":["Read sources"],"findings":[],"artifacts":[],"verification":[],"remaining":[]}"#).unwrap()
    }
    #[test]
    fn request_and_result_validation_rejects_unbounded_or_ambiguous_data() {
        let mut input = request();
        input.tools.push("read_file".into());
        assert!(input.validate().is_err());
        input = request();
        input.task = " ".into();
        assert!(input.validate().is_err());
        input.task = "x".repeat(65537);
        assert!(input.validate().is_err());
        assert!(AgentResult::parse(r#"{"outcome":"done","summary":" ","work_done":[],"findings":[],"artifacts":[],"verification":[],"remaining":[]}"#).is_err());
        assert!(AgentResult::parse(r#"{"outcome":"done","summary":"ok","work_done":[],"findings":[],"artifacts":[],"verification":[],"remaining":[],"extra":1}"#).is_err());
        assert_eq!(result().summary, "Inspected sources");
    }
    #[test]
    fn isolation_and_cancellation_preserve_active_worker_until_exit() {
        let manager = AgentRunManager::default();
        let (release, gate) = mpsc::channel::<()>();
        let (entered, ready) = mpsc::channel::<()>();
        let run = manager
            .start("a", request(), Duration::from_secs(5), move |control| {
                entered.send(()).unwrap();
                gate.recv().unwrap();
                assert!(control.cancelled());
                Ok(result())
            })
            .unwrap();
        ready.recv_timeout(Duration::from_secs(2)).unwrap();
        assert!(manager.read("b", &run.run_id).is_err());
        assert!(manager.cancel("b", &run.run_id).is_err());
        assert!(manager.list("b").unwrap().is_empty());
        assert_eq!(
            manager.cancel("a", &run.run_id).unwrap().status,
            AgentRunStatus::Cancelling
        );
        assert_eq!(
            manager.read("a", &run.run_id).unwrap().status,
            AgentRunStatus::Cancelling
        );
        release.send(()).unwrap();
        let ended = manager
            .wait("a", &run.run_id, Duration::from_secs(2))
            .unwrap();
        assert_eq!(ended.status, AgentRunStatus::Cancelled);
        assert!(ended.result.is_none());
    }
    #[test]
    fn timeout_is_terminal_only_after_worker_exits() {
        let manager = AgentRunManager::default();
        let (release, gate) = mpsc::channel::<()>();
        let (entered, ready) = mpsc::channel::<()>();
        let run = manager
            .start("a", request(), Duration::from_millis(100), move |control| {
                entered.send(()).unwrap();
                gate.recv().unwrap();
                assert!(control.check().is_err());
                Ok(result())
            })
            .unwrap();
        ready.recv_timeout(Duration::from_secs(2)).unwrap();
        std::thread::sleep(Duration::from_millis(120));
        assert!(!manager.read("a", &run.run_id).unwrap().status.is_terminal());
        release.send(()).unwrap();
        assert_eq!(
            manager
                .wait("a", &run.run_id, Duration::from_secs(2))
                .unwrap()
                .status,
            AgentRunStatus::TimedOut
        );
    }
    #[test]
    fn atomic_caps_and_closed_owners_reject_starts() {
        let manager = AgentRunManager::default();
        let mut releases = Vec::new();
        for owner in ["a", "b", "c", "d"] {
            for _ in 0..4 {
                let (release, gate) = mpsc::channel::<()>();
                releases.push(release);
                manager
                    .start(owner, request(), Duration::from_secs(5), move |_| {
                        let _ = gate.recv();
                        Ok(result())
                    })
                    .unwrap();
            }
        }
        assert!(manager
            .start("a", request(), Duration::from_secs(5), |_| Ok(result()))
            .is_err());
        assert!(manager
            .start("e", request(), Duration::from_secs(5), |_| Ok(result()))
            .is_err());
        manager.close_owner("a");
        assert!(manager
            .start("a", request(), Duration::from_secs(5), |_| Ok(result()))
            .is_err());
        manager.shutdown();
        drop(releases);
    }
    #[test]
    fn retention_evicts_finished_runs_without_losing_active_runs() {
        let manager = AgentRunManager::default();
        let (release, gate) = mpsc::channel::<()>();
        let active = manager
            .start("a", request(), Duration::from_secs(60), move |_| {
                let _ = gate.recv();
                Ok(result())
            })
            .unwrap();
        for _ in 0..40 {
            let run = manager
                .start("a", request(), Duration::from_secs(5), |_| Ok(result()))
                .unwrap();
            assert_eq!(
                manager
                    .wait("a", &run.run_id, Duration::from_secs(2))
                    .unwrap()
                    .status,
                AgentRunStatus::Completed
            );
        }
        assert_eq!(manager.list("a").unwrap().len(), 32);
        assert!(manager.read("a", &active.run_id).is_ok());
        for owner in ["b", "c", "d", "e"] {
            for _ in 0..32 {
                let run = manager
                    .start(owner, request(), Duration::from_secs(5), |_| Ok(result()))
                    .unwrap();
                manager
                    .wait(owner, &run.run_id, Duration::from_secs(2))
                    .unwrap();
            }
        }
        let count: usize = ["a", "b", "c", "d", "e"]
            .iter()
            .map(|owner| manager.list(owner).unwrap().len())
            .sum();
        assert_eq!(count, 128);
        assert!(manager.read("a", &active.run_id).is_ok());
        drop(release);
    }
    #[test]
    fn progress_counters_approval_and_panics_are_bounded() {
        let manager = AgentRunManager::default();
        let (release, gate) = mpsc::channel::<()>();
        let (ready, receive) = mpsc::channel::<()>();
        let run = manager
            .start("a", request(), Duration::from_secs(5), move |control| {
                control.update(Some(&"é".repeat(5000)), Some("read_file"));
                control.increment_tool_calls();
                control.waiting_approval(true);
                ready.send(()).unwrap();
                let _ = gate.recv();
                panic!("private panic payload");
            })
            .unwrap();
        receive.recv().unwrap();
        let view = manager.read("a", &run.run_id).unwrap();
        assert_eq!(view.status, AgentRunStatus::WaitingApproval);
        assert_eq!(view.tool_calls, 1);
        assert!(view.progress.unwrap().len() <= 4096);
        release.send(()).unwrap();
        let ended = manager
            .wait("a", &run.run_id, Duration::from_secs(2))
            .unwrap();
        assert_eq!(ended.status, AgentRunStatus::Failed);
        assert!(!ended.error.unwrap().contains("private panic"));
    }
    #[test]
    fn approvals_require_matching_owner_and_explicit_decision_and_cancel_wakes_waiter() {
        use crate::permissions::{AuthorizationDecision, AuthorizationRequest};
        let manager = AgentRunManager::default();
        let approval = AuthorizationRequest {
            id: "approval".into(),
            tool: "exec_command".into(),
            arguments: "{}".into(),
            reason: "command".into(),
        };
        let run = manager
            .start("a", request(), Duration::from_secs(5), move |control| {
                let decision = control.authorize(&approval)?;
                assert_eq!(decision, AuthorizationDecision::Deny);
                control.record_tool("exec_command", Some(false), "Denied");
                Ok(result())
            })
            .unwrap();
        let until = Instant::now() + Duration::from_secs(2);
        loop {
            assert!(Instant::now() < until, "approval was not published");
            if manager
                .read("a", &run.run_id)
                .unwrap()
                .pending_approval
                .is_some()
            {
                break;
            }
            std::thread::sleep(Duration::from_millis(1));
        }
        assert!(manager
            .resolve_approval("b", &run.run_id, "approval", true)
            .is_err());
        assert!(manager
            .resolve_approval("a", &run.run_id, "wrong", true)
            .is_err());
        manager
            .resolve_approval("a", &run.run_id, "approval", false)
            .unwrap();
        assert!(manager
            .resolve_approval("a", &run.run_id, "approval", true)
            .is_err());
        let ended = manager
            .wait("a", &run.run_id, Duration::from_secs(2))
            .unwrap();
        assert!(ended.pending_approval.is_none());
        assert_eq!(ended.activity[0].summary, "Denied");
        let approval = AuthorizationRequest {
            id: "second".into(),
            tool: "exec_command".into(),
            arguments: "{}".into(),
            reason: "command".into(),
        };
        let run = manager
            .start("a", request(), Duration::from_secs(5), move |control| {
                control.authorize(&approval)?;
                Ok(result())
            })
            .unwrap();
        let until = Instant::now() + Duration::from_secs(2);
        loop {
            assert!(Instant::now() < until, "approval was not published");
            if manager
                .read("a", &run.run_id)
                .unwrap()
                .pending_approval
                .is_some()
            {
                break;
            }
            std::thread::sleep(Duration::from_millis(1));
        }
        let cancelling = manager.cancel("a", &run.run_id).unwrap();
        assert!(cancelling.pending_approval.is_none());
        assert!(manager
            .resolve_approval("a", &run.run_id, "second", true)
            .is_err());
        assert_eq!(
            manager
                .wait("a", &run.run_id, Duration::from_secs(2))
                .unwrap()
                .status,
            AgentRunStatus::Cancelled
        );
    }
    #[test]
    fn adoption_moves_authority_and_closes_source_without_cancelling_workers() {
        let manager = AgentRunManager::default();
        let (release, gate) = mpsc::channel::<()>();
        let run = manager
            .start("draft", request(), Duration::from_secs(5), move |control| {
                gate.recv().unwrap();
                control.check()?;
                Ok(result())
            })
            .unwrap();
        manager.adopt_owner("draft", "saved").unwrap();
        assert!(manager.read("draft", &run.run_id).is_err());
        assert_eq!(
            manager.read("saved", &run.run_id).unwrap().status,
            AgentRunStatus::Running
        );
        assert!(manager
            .start("draft", request(), Duration::from_secs(5), |_| Ok(result()))
            .is_err());
        release.send(()).unwrap();
        assert_eq!(
            manager
                .wait("saved", &run.run_id, Duration::from_secs(2))
                .unwrap()
                .status,
            AgentRunStatus::Completed
        );
        manager.close_owner("closed");
        assert!(manager.adopt_owner("saved", "closed").is_err());
        assert!(manager.read("saved", &run.run_id).is_ok());
    }
    #[test]
    fn adoption_rejects_overfull_merge_without_partial_transfer() {
        let manager = AgentRunManager::default();
        let mut releases = Vec::new();
        let mut draft_id = String::new();
        for owner in ["draft", "saved", "saved", "saved", "saved"] {
            let (release, gate) = mpsc::channel::<()>();
            releases.push(release);
            let run = manager
                .start(owner, request(), Duration::from_secs(5), move |_| {
                    let _ = gate.recv();
                    Ok(result())
                })
                .unwrap();
            if owner == "draft" {
                draft_id = run.run_id;
            }
        }
        assert!(manager.adopt_owner("draft", "saved").is_err());
        assert!(manager.read("draft", &draft_id).is_ok());
        assert!(manager.read("saved", &draft_id).is_err());
        drop(releases);
    }
    #[test]
    fn cancelling_freezes_worker_activity_and_clears_pending_approval() {
        let manager = AgentRunManager::default();
        let (ready, entered) = mpsc::channel::<()>();
        let (release, gate) = mpsc::channel::<()>();
        let run = manager
            .start("a", request(), Duration::from_secs(5), move |control| {
                control.update(Some("Before cancellation"), Some("read_file"));
                ready.send(()).unwrap();
                gate.recv().unwrap();
                control.update(Some("Late update"), Some("exec_command"));
                control.record_tool("exec_command", Some(true), "Late result");
                control.increment_tool_calls();
                control.waiting_approval(true);
                Ok(result())
            })
            .unwrap();
        entered.recv_timeout(Duration::from_secs(2)).unwrap();
        manager.cancel("a", &run.run_id).unwrap();
        release.send(()).unwrap();
        let ended = manager
            .wait("a", &run.run_id, Duration::from_secs(2))
            .unwrap();
        assert_eq!(ended.status, AgentRunStatus::Cancelled);
        assert_eq!(ended.progress.as_deref(), Some("Before cancellation"));
        assert!(ended.activity.is_empty());
        assert_eq!(ended.tool_calls, 0);
        assert!(ended.pending_approval.is_none());
    }
    #[test]
    fn model_usage_snapshots_replace_without_aggregation_and_freeze_after_cancel() {
        use crate::models::ModelUsage;
        let manager = AgentRunManager::default();
        let (send_control, receive_control) = mpsc::channel();
        let (release, gate) = mpsc::channel::<()>();
        let run = manager
            .start("a", request(), Duration::from_secs(5), move |control| {
                send_control.send(control).unwrap();
                gate.recv().unwrap();
                Ok(result())
            })
            .unwrap();
        let control: RunControl = receive_control
            .recv_timeout(Duration::from_secs(2))
            .unwrap();
        control.set_model(&"é".repeat(200));
        assert!(manager.read("a", &run.run_id).unwrap().model.unwrap().len() <= 128);
        control.set_model("requested-model");
        let usage = ModelUsage {
            source: None,
            model: "requested-model".into(),
            input_tokens: 5,
            output_tokens: 3,
            total_tokens: 8,
        };
        control.record_usage(Some(&usage));
        control.record_usage(Some(&usage));
        assert_eq!(
            manager.read("a", &run.run_id).unwrap().usage,
            Some(usage.clone())
        );
        control.record_usage(None);
        assert!(manager.read("a", &run.run_id).unwrap().usage.is_none());
        control.record_usage(Some(&usage));
        manager.cancel("a", &run.run_id).unwrap();
        control.set_model("late-model");
        control.record_usage(None);
        let cancelling = manager.read("a", &run.run_id).unwrap();
        assert_eq!(cancelling.model.as_deref(), Some("requested-model"));
        assert_eq!(cancelling.usage, Some(usage.clone()));
        release.send(()).unwrap();
        manager
            .wait("a", &run.run_id, Duration::from_secs(2))
            .unwrap();
        control.set_model("terminal-model");
        control.record_usage(None);
        let terminal = manager.read("a", &run.run_id).unwrap();
        assert_eq!(terminal.model.as_deref(), Some("requested-model"));
        assert_eq!(terminal.usage, Some(usage));
    }
    #[test]
    fn approval_payload_bound_rejects_oversized_arguments_without_publishing() {
        let manager = AgentRunManager::default();
        let run = manager
            .start("a", request(), Duration::from_secs(5), move |control| {
                let request = AuthorizationRequest {
                    id: "approval".into(),
                    tool: "exec_command".into(),
                    arguments: "x".repeat(8193),
                    reason: "command".into(),
                };
                assert!(control.authorize(&request).is_err());
                Ok(result())
            })
            .unwrap();
        let ended = manager
            .wait("a", &run.run_id, Duration::from_secs(2))
            .unwrap();
        assert_eq!(ended.status, AgentRunStatus::Completed);
        assert!(ended.pending_approval.is_none());
    }
    #[test]
    fn cancelling_parent_cancels_descendants_but_preserves_unrelated_run() {
        let manager = AgentRunManager::default();
        let mut releases = Vec::new();
        let mut start = |parent: Option<&str>| {
            let (release, gate) = mpsc::channel::<()>();
            releases.push(release);
            let (entered, ready) = mpsc::channel::<()>();
            let run = manager
                .start_with_parent("a", request(), Duration::from_secs(5), parent, move |_| {
                    entered.send(()).unwrap();
                    let _ = gate.recv();
                    Ok(result())
                })
                .unwrap();
            ready.recv_timeout(Duration::from_secs(2)).unwrap();
            run
        };
        let parent = start(None);
        let child = start(Some(&parent.run_id));
        let grandchild = start(Some(&child.run_id));
        let unrelated = start(None);
        assert_eq!(child.parent_run_id.as_deref(), Some(parent.run_id.as_str()));
        manager.cancel("a", &parent.run_id).unwrap();
        assert_eq!(
            manager.read("a", &child.run_id).unwrap().status,
            AgentRunStatus::Cancelling
        );
        assert_eq!(
            manager.read("a", &grandchild.run_id).unwrap().status,
            AgentRunStatus::Cancelling
        );
        assert_eq!(
            manager.read("a", &unrelated.run_id).unwrap().status,
            AgentRunStatus::Running
        );
        assert!(manager
            .start_with_parent(
                "a",
                request(),
                Duration::from_secs(5),
                Some(&parent.run_id),
                |_| Ok(result())
            )
            .is_err());
        drop(releases);
    }
    #[test]
    fn failed_parent_cancels_children_and_parent_authority_is_scoped() {
        let manager = AgentRunManager::default();
        let (release_parent, parent_gate) = mpsc::channel::<()>();
        let parent = manager
            .start("a", request(), Duration::from_secs(5), move |control| {
                assert!(!control.run_id().is_empty());
                parent_gate.recv().unwrap();
                Err("worker failure".into())
            })
            .unwrap();
        assert!(manager
            .start_with_parent(
                "b",
                request(),
                Duration::from_secs(5),
                Some(&parent.run_id),
                |_| Ok(result())
            )
            .is_err());
        let (release_child, child_gate) = mpsc::channel::<()>();
        let (entered, ready) = mpsc::channel::<()>();
        let child = manager
            .start_with_parent(
                "a",
                request(),
                Duration::from_secs(5),
                Some(&parent.run_id),
                move |_| {
                    entered.send(()).unwrap();
                    let _ = child_gate.recv();
                    Ok(result())
                },
            )
            .unwrap();
        ready.recv_timeout(Duration::from_secs(2)).unwrap();
        release_parent.send(()).unwrap();
        assert_eq!(
            manager
                .wait("a", &parent.run_id, Duration::from_secs(2))
                .unwrap()
                .status,
            AgentRunStatus::Failed
        );
        assert_eq!(
            manager.read("a", &child.run_id).unwrap().status,
            AgentRunStatus::Cancelling
        );
        drop(release_child);
    }
}
