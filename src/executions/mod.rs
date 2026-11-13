//! Owned tool workers and metadata-only Wakeup mailboxes. No provider calls occur here.
mod output;
pub mod protocol;
pub mod worker;
use crate::tools::{ToolAttachment, ToolResult};
pub use output::OutputRead;
use serde::Serialize;
use std::{
    cmp::Reverse,
    collections::{BinaryHeap, HashMap, HashSet},
    sync::{Arc, Condvar, Mutex},
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};
const ACTIVE_OWNER: usize = 8;
const ACTIVE_TOTAL: usize = 32;
const RECORDS_OWNER: usize = 32;
const RECORDS_TOTAL: usize = 128;
const BATCH: usize = 16;
#[derive(Clone)]
pub struct ToolExecutionManager(Arc<Coordinator>);
struct Coordinator {
    shared: Arc<Shared>,
    scheduler: Mutex<Option<JoinHandle<()>>>,
}
struct Shared {
    store: Mutex<Store>,
    changed: Condvar,
}
#[derive(Default)]
struct Store {
    records: HashMap<String, Record>,
    mailboxes: HashMap<String, Mailbox>,
    closed: HashSet<String>,
    deadlines: BinaryHeap<Reverse<(Instant, u64, String, String)>>,
    schedules: HashMap<(String, String), (Instant, u64)>,
    sequence: u64,
    shutdown: bool,
    owner_deadlines: HashMap<String, Instant>,
}
#[derive(Default)]
struct Mailbox {
    pending: HashMap<String, WakeupEvent>,
    in_flight: Option<WakeupBatch>,
    consumer: bool,
    image_epoch: u64,
}
struct Record {
    owner: String,
    tool: String,
    status: &'static str,
    started: Instant,
    revision: u64,
    output: output::Output,
    metadata: serde_json::Value,
    error: Option<String>,
    cancel_requested: bool,
    cancel_ack: bool,
    structured: bool,
    image_epoch: u64,
    attachments: Vec<ToolAttachment>,
    worker: Option<JoinHandle<()>>,
    launched: bool,
}
#[derive(Clone)]
pub struct ExecutionScope {
    manager: ToolExecutionManager,
    owner: String,
}
pub struct ConsumerLease {
    shared: Arc<Shared>,
    owner: String,
}
pub struct ExecutionReporter {
    shared: Arc<Shared>,
    id: String,
}
#[derive(Debug, Clone, Serialize)]
pub struct ExecutionInfo {
    pub execution_id: String,
    pub tool: String,
    pub status: String,
    pub elapsed_ms: u64,
    pub revision: u64,
    pub unread_lines: usize,
    pub retention_loss: bool,
    pub cancel_requested: bool,
    pub has_image: bool,
}
#[derive(Debug, Clone, Serialize)]
pub struct WakeupEvent {
    pub execution_id: String,
    pub tool: String,
    pub status: String,
    pub reason: String,
    pub revision: u64,
    pub elapsed_ms: u64,
    pub unread_lines: usize,
    pub retention_loss: bool,
    #[serde(skip)]
    sequence: u64,
}
#[derive(Debug, Clone, Serialize)]
pub struct WakeupBatch {
    #[serde(rename = "type")]
    kind: &'static str,
    pub batch_id: String,
    pub events: Vec<WakeupEvent>,
    pub pending_count: usize,
}
#[derive(Serialize)]
pub struct ExecutionRead {
    pub execution: ExecutionInfo,
    #[serde(flatten)]
    pub output: OutputRead,
    pub metadata: serde_json::Value,
    pub error: Option<String>,
    pub format: &'static str,
}
impl std::ops::Deref for ExecutionRead {
    type Target = OutputRead;
    fn deref(&self) -> &Self::Target {
        &self.output
    }
}
fn owner_closed(store: &Store, owner: &str) -> bool {
    store.shutdown
        || store
            .closed
            .iter()
            .any(|closed| owner == closed || owner.starts_with(&format!("{closed}/")))
}
fn close_store_owner(store: &mut Store, owner: &str) {
    let owners = store
        .mailboxes
        .keys()
        .chain(store.records.values().map(|r| &r.owner))
        .filter(|id| id.as_str() == owner || id.starts_with(&format!("{owner}/")))
        .cloned()
        .collect::<HashSet<_>>();
    for id in owners {
        store.closed.insert(id.clone());
        store.mailboxes.remove(&id);
    }
    store.closed.insert(owner.into());
    for r in store
        .records
        .values_mut()
        .filter(|r| r.owner == owner || r.owner.starts_with(&format!("{owner}/")))
    {
        if !terminal(r.status) {
            r.cancel_requested = true;
            r.status = "cancelling";
        }
        r.attachments.clear();
        r.output = output::Output::default();
        r.metadata = serde_json::json!({});
        r.error = None;
    }
    store
        .schedules
        .retain(|(id, _), _| id != owner && !id.starts_with(&format!("{owner}/")));
    store
        .owner_deadlines
        .retain(|id, _| id != owner && !id.starts_with(&format!("{owner}/")));
}
fn terminal(status: &str) -> bool {
    matches!(status, "succeeded" | "failed" | "cancelled")
}
fn elapsed(start: Instant) -> u64 {
    start.elapsed().as_millis().min(u64::MAX as u128) as u64
}
fn info(id: &str, r: &Record) -> ExecutionInfo {
    ExecutionInfo {
        execution_id: id.into(),
        tool: r.tool.clone(),
        status: r.status.into(),
        elapsed_ms: elapsed(r.started),
        revision: r.revision,
        unread_lines: r.output.unread_lines(),
        retention_loss: r.output.retention_loss(),
        cancel_requested: r.cancel_requested,
        has_image: !r.attachments.is_empty(),
    }
}
fn enqueue(store: &mut Store, id: &str, reason: &str) {
    let Some(r) = store.records.get(id) else {
        return;
    };
    if owner_closed(store, &r.owner) {
        return;
    }
    let event = WakeupEvent {
        execution_id: id.into(),
        tool: r.tool.clone(),
        status: r.status.into(),
        reason: reason.into(),
        revision: r.revision,
        elapsed_ms: elapsed(r.started),
        unread_lines: r.output.unread_lines(),
        retention_loss: r.output.retention_loss(),
        sequence: store.sequence,
    };
    store.sequence += 1;
    let box_ = store.mailboxes.entry(r.owner.clone()).or_default();
    if box_
        .pending
        .get(id)
        .is_some_and(|e| terminal(&e.status) && !terminal(&event.status))
    {
        return;
    }
    box_.pending.insert(id.into(), event);
}
fn claim(store: &mut Store, owner: &str) -> Option<WakeupBatch> {
    let box_ = store.mailboxes.entry(owner.into()).or_default();
    if let Some(batch) = &box_.in_flight {
        return Some(batch.clone());
    }
    if box_.pending.is_empty() {
        return None;
    }
    let mut events = box_.pending.values().cloned().collect::<Vec<_>>();
    events.sort_by_key(|e| {
        (
            if e.reason == "interval_elapsed" { 1 } else { 0 },
            e.sequence,
        )
    });
    events.truncate(BATCH);
    for event in &events {
        box_.pending.remove(&event.execution_id);
    }
    let batch = WakeupBatch {
        kind: "execution_wakeup",
        batch_id: uuid::Uuid::new_v4().to_string(),
        events,
        pending_count: box_.pending.len(),
    };
    box_.in_flight = Some(batch.clone());
    Some(batch)
}
fn validate_wait(ms: u64) -> Result<(), String> {
    if !(100..30000).contains(&ms) {
        Err("Wakeup sleep must be 100–29999ms (strictly below 30 seconds)".into())
    } else {
        Ok(())
    }
}
impl Default for ToolExecutionManager {
    fn default() -> Self {
        Self::new()
    }
}
impl ToolExecutionManager {
    pub fn new() -> Self {
        let shared = Arc::new(Shared {
            store: Mutex::new(Store::default()),
            changed: Condvar::new(),
        });
        let scheduler_shared = shared.clone();
        let scheduler = thread::Builder::new()
            .name("atlas-wakeup".into())
            .spawn(move || scheduler_loop(scheduler_shared))
            .ok();
        Self(Arc::new(Coordinator {
            shared,
            scheduler: Mutex::new(scheduler),
        }))
    }
    pub fn scope(&self, owner: &str) -> ExecutionScope {
        ExecutionScope {
            manager: self.clone(),
            owner: owner.into(),
        }
    }
    pub fn check_adoption(&self, from: &str, to: &str) -> Result<(), String> {
        if from == to {
            return Ok(());
        }
        let s = self
            .0
            .shared
            .store
            .lock()
            .map_err(|_| "Execution manager unavailable")?;
        if owner_closed(&s, from) || owner_closed(&s, to) || s.shutdown {
            return Err("Execution owner is closed".into());
        }
        if s.mailboxes.iter().any(|(id, b)| {
            b.consumer
                && (id == from
                    || id == to
                    || id.starts_with(&format!("{from}/"))
                    || id.starts_with(&format!("{to}/")))
        }) {
            return Err("Cannot merge an active Wakeup consumer".into());
        }
        if s.records.values().any(|r| {
            (r.owner == from
                || r.owner == to
                || r.owner.starts_with(&format!("{from}/"))
                || r.owner.starts_with(&format!("{to}/")))
                && !terminal(r.status)
        }) {
            return Err("Cannot merge native scopes while tool executions are active".into());
        }
        if s.records
            .values()
            .filter(|r| r.owner == from || r.owner == to)
            .count()
            > RECORDS_OWNER
        {
            return Err("Merged execution history exceeds retention limit".into());
        }
        Ok(())
    }
    pub fn adopt_owner(&self, from: &str, to: &str) -> Result<(), String> {
        self.check_adoption(from, to)?;
        let mut s = self
            .0
            .shared
            .store
            .lock()
            .map_err(|_| "Execution manager unavailable")?;
        if from == to {
            return Ok(());
        }
        if s.mailboxes.iter().any(|(id, b)| {
            b.consumer
                && (id == from
                    || id == to
                    || id.starts_with(&format!("{from}/"))
                    || id.starts_with(&format!("{to}/")))
        }) {
            return Err("Cannot merge an active Wakeup consumer".into());
        }
        if s.records
            .values()
            .filter(|r| r.owner == from || r.owner == to)
            .count()
            > RECORDS_OWNER
        {
            return Err("Merged execution history exceeds retention limit".into());
        }
        if owner_closed(&s, from) || owner_closed(&s, to) || s.shutdown {
            return Err("Execution owner is closed".into());
        }
        if s.records.values().any(|r| {
            (r.owner == from
                || r.owner == to
                || r.owner.starts_with(&format!("{from}/"))
                || r.owner.starts_with(&format!("{to}/")))
                && !terminal(r.status)
        }) {
            return Err("Cannot merge native scopes while tool executions are active".into());
        }
        for r in s.records.values_mut() {
            if r.owner == from {
                r.owner = to.into();
            } else if let Some(suffix) = r.owner.strip_prefix(&format!("{from}/")) {
                r.owner = format!("{to}/{suffix}");
            }
        }
        if let Some(old) = s.mailboxes.remove(from) {
            let target = s.mailboxes.entry(to.into()).or_default();
            target.pending.extend(old.pending);
            if let Some(batch) = old.in_flight {
                for event in batch.events {
                    target.pending.insert(event.execution_id.clone(), event);
                }
            }
        }
        let closed = s
            .closed
            .iter()
            .filter_map(|id| {
                id.strip_prefix(&format!("{from}/"))
                    .map(|suffix| format!("{to}/{suffix}"))
            })
            .collect::<Vec<_>>();
        s.closed.extend(closed);
        s.closed.insert(from.into());
        Ok(())
    }
    pub fn close_owner(&self, owner: &str) {
        let mut store = self
            .0
            .shared
            .store
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        close_store_owner(&mut store, owner);
        self.0.shared.changed.notify_all();
    }
    pub fn shutdown(&self, timeout: Duration) -> Result<(), String> {
        let shared = &self.0.shared;
        let end = Instant::now() + timeout;
        let mut store = shared
            .store
            .lock()
            .map_err(|_| "Execution manager unavailable")?;
        store.shutdown = true;
        for r in store.records.values_mut() {
            if !terminal(r.status) {
                r.cancel_requested = true;
                r.status = "cancelling";
            }
        }
        shared.changed.notify_all();
        while store.records.values().any(|r| !terminal(r.status)) {
            let remain = end.saturating_duration_since(Instant::now());
            if remain.is_zero() {
                return Err(
                    "Execution cleanup is still pending; action outcomes may be unknown".into(),
                );
            }
            store = shared
                .changed
                .wait_timeout(store, remain)
                .map_err(|_| "Execution manager unavailable")?
                .0;
        }
        Ok(())
    }
}
impl ExecutionScope {
    pub fn child(&self, id: &str) -> Self {
        self.manager.scope(&format!("{}/{}", self.owner, id))
    }
    pub fn close(&self) {
        self.manager.close_owner(&self.owner);
    }
    pub(crate) fn set_deadline(&self, deadline: Instant) {
        let mut s = self
            .manager
            .0
            .shared
            .store
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        s.owner_deadlines.insert(self.owner.clone(), deadline);
        self.manager.0.shared.changed.notify_all();
    }
    pub(crate) fn check_open(&self) -> Result<(), String> {
        let s = self
            .manager
            .0
            .shared
            .store
            .lock()
            .map_err(|_| "Execution manager unavailable")?;
        if owner_closed(&s, &self.owner) {
            Err("Execution owner is closed".into())
        } else {
            Ok(())
        }
    }
    pub fn owner(&self) -> &str {
        &self.owner
    }
    pub fn manager(&self) -> ToolExecutionManager {
        self.manager.clone()
    }
    pub fn acquire_consumer(&self) -> Result<ConsumerLease, String> {
        let mut s = self
            .manager
            .0
            .shared
            .store
            .lock()
            .map_err(|_| "Execution manager unavailable")?;
        if s.shutdown || owner_closed(&s, &self.owner) {
            return Err("Execution owner is closed".into());
        }
        let b = s.mailboxes.entry(self.owner.clone()).or_default();
        if b.consumer {
            return Err("Agent already has an active Wakeup consumer".into());
        }
        b.consumer = true;
        b.image_epoch = b.image_epoch.wrapping_add(1);
        Ok(ConsumerLease {
            shared: self.manager.0.shared.clone(),
            owner: self.owner.clone(),
        })
    }
    pub fn start<F>(&self, tool: &str, work: F) -> Result<ExecutionInfo, String>
    where
        F: FnOnce(ExecutionReporter) -> Result<ToolResult, String> + Send + 'static,
    {
        let slot = self.reserve(tool)?;
        self.launch(&slot.execution_id, work)
    }
    pub(crate) fn reserve(&self, tool: &str) -> Result<ExecutionInfo, String> {
        if self
            .manager
            .0
            .scheduler
            .lock()
            .map_err(|_| "Wakeup scheduler unavailable")?
            .is_none()
        {
            return Err("Wakeup scheduler unavailable".into());
        }
        if tool.len() > 128 {
            return Err("Tool name exceeds128bytes".into());
        }
        let shared = self.manager.0.shared.clone();
        let id = uuid::Uuid::new_v4().to_string();
        let mut s = shared
            .store
            .lock()
            .map_err(|_| "Execution manager unavailable")?;
        if s.shutdown || owner_closed(&s, &self.owner) {
            return Err("Execution owner is closed".into());
        }
        if s.records.values().filter(|r| !terminal(r.status)).count() >= ACTIVE_TOTAL
            || s.records
                .values()
                .filter(|r| r.owner == self.owner && !terminal(r.status))
                .count()
                >= ACTIVE_OWNER
        {
            return Err("Concurrent tool execution limit reached".into());
        }
        while s.records.len() >= RECORDS_TOTAL
            || s.records.values().filter(|r| r.owner == self.owner).count() >= RECORDS_OWNER
        {
            let per_owner =
                s.records.values().filter(|r| r.owner == self.owner).count() >= RECORDS_OWNER;
            let old = s
                .records
                .iter()
                .filter(|(id, r)| {
                    terminal(r.status)
                        && (!per_owner || r.owner == self.owner)
                        && !s.mailboxes.values().any(|b| {
                            b.pending.contains_key(*id)
                                || b.in_flight.as_ref().is_some_and(|batch| {
                                    batch.events.iter().any(|e| &e.execution_id == *id)
                                })
                        })
                })
                .min_by_key(|(_, r)| r.started)
                .map(|(id, _)| id.clone());
            let Some(old) = old else {
                return Err("Execution retention is full; consume pending wakeups first".into());
            };
            s.records.remove(&old);
        }
        let image_epoch = s
            .mailboxes
            .get(&self.owner)
            .map(|b| b.image_epoch)
            .unwrap_or(0);
        s.records.insert(
            id.clone(),
            Record {
                owner: self.owner.clone(),
                tool: tool.into(),
                status: "running",
                started: Instant::now(),
                revision: 0,
                output: output::Output::default(),
                metadata: serde_json::json!({}),
                error: None,
                cancel_requested: false,
                cancel_ack: false,
                structured: false,
                image_epoch,
                attachments: vec![],
                worker: None,
                launched: false,
            },
        );
        let result = info(&id, s.records.get(&id).unwrap());
        drop(s);
        crate::diagnostics::event(
            "INFO",
            "execution.accepted",
            serde_json::json!({"executionId":id,"tool":tool}),
        );
        Ok(result)
    }
    pub(crate) fn fail_reserved(&self, id: &str, error: String) {
        ExecutionReporter {
            shared: self.manager.0.shared.clone(),
            id: id.into(),
        }
        .finish(Err(error));
    }
    pub(crate) fn launch<F>(&self, id: &str, work: F) -> Result<ExecutionInfo, String>
    where
        F: FnOnce(ExecutionReporter) -> Result<ToolResult, String> + Send + 'static,
    {
        let shared = self.manager.0.shared.clone();
        let id = id.to_string();
        let mut s = shared
            .store
            .lock()
            .map_err(|_| "Execution manager unavailable")?;
        let record = owned(&s, &self.owner, &id)?;
        if record.launched || terminal(record.status) {
            return Err("Execution was already launched".into());
        }
        s.records.get_mut(&id).unwrap().launched = true;
        drop(s);
        let reporter = ExecutionReporter {
            shared: shared.clone(),
            id: id.clone(),
        };
        let worker = thread::Builder::new()
            .name("atlas-tool".into())
            .spawn(move || {
                if reporter.cancelled() {
                    reporter.ack_cancel();
                    reporter.finish(Err("Execution cancelled before start".into()));
                    return;
                }
                let handle = ExecutionReporter {
                    shared: reporter.shared.clone(),
                    id: reporter.id.clone(),
                };
                let mut result = Err("Tool worker did not finish".to_string());
                let _ = crate::diagnostics::run(
                    "execution.worker",
                    serde_json::json!({"executionId":reporter.id}),
                    || {
                        result =
                            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| work(handle)))
                                .unwrap_or_else(|_| {
                                    Err("Tool worker panicked; action outcome may be unknown"
                                        .into())
                                });
                        result
                            .as_ref()
                            .map(|_| ())
                            .map_err(|_| "tool execution failed".to_string())
                    },
                );
                reporter.finish(result);
            })
            .map_err(|_| {
                self.fail_reserved(
                    &id,
                    "Could not start tool worker; prepared actions may need inspection".into(),
                );
                "Could not start tool worker"
            })?;
        let mut s = shared
            .store
            .lock()
            .map_err(|_| "Execution manager unavailable")?;
        let r = s
            .records
            .get_mut(&id)
            .ok_or("Execution was closed during launch")?;
        r.worker = Some(worker);
        Ok(info(&id, r))
    }
    pub fn info(&self, id: &str) -> Result<ExecutionInfo, String> {
        let s = self
            .manager
            .0
            .shared
            .store
            .lock()
            .map_err(|_| "Execution manager unavailable")?;
        let r = owned(&s, &self.owner, id)?;
        Ok(info(id, r))
    }
    pub fn list(&self) -> Result<Vec<ExecutionInfo>, String> {
        self.check_open()?;
        let s = self
            .manager
            .0
            .shared
            .store
            .lock()
            .map_err(|_| "Execution manager unavailable")?;
        Ok(s.records
            .iter()
            .filter(|(_, r)| r.owner == self.owner)
            .map(|(id, r)| info(id, r))
            .collect())
    }
    pub fn claim(&self) -> Result<Option<WakeupBatch>, String> {
        let mut s = self
            .manager
            .0
            .shared
            .store
            .lock()
            .map_err(|_| "Execution manager unavailable")?;
        if s.shutdown || owner_closed(&s, &self.owner) {
            return Err("Execution owner is closed".into());
        }
        Ok(claim(&mut s, &self.owner))
    }
    pub fn ack(&self, batch: &str) -> Result<(), String> {
        let mut s = self
            .manager
            .0
            .shared
            .store
            .lock()
            .map_err(|_| "Execution manager unavailable")?;
        let b = s.mailboxes.entry(self.owner.clone()).or_default();
        if b.in_flight.as_ref().is_none_or(|v| v.batch_id != batch) {
            return Err("Wakeup batch is not in flight for this owner".into());
        }
        b.in_flight = None;
        Ok(())
    }
    pub fn wait(&self, ids: &[String], ms: u64) -> Result<WakeupBatch, String> {
        self.wait_checked(ids, ms, || Ok(()))
    }
    pub(crate) fn wait_checked(
        &self,
        ids: &[String],
        ms: u64,
        mut check: impl FnMut() -> Result<(), String>,
    ) -> Result<WakeupBatch, String> {
        validate_wait(ms)?;
        if ids.is_empty() || ids.len() > 32 {
            return Err("Wait for 1–32 owned executions".into());
        }
        let shared = &self.manager.0.shared;
        let mut s = shared
            .store
            .lock()
            .map_err(|_| "Execution manager unavailable")?;
        for id in ids {
            owned(&s, &self.owner, id)?;
        }
        if let Some(batch) = claim(&mut s, &self.owner) {
            return Ok(batch);
        }
        for id in ids {
            if terminal(s.records[id].status) {
                enqueue(&mut s, id, "snapshot");
            } else {
                s.sequence += 1;
                let seq = s.sequence;
                let at = Instant::now() + Duration::from_millis(ms);
                s.schedules
                    .insert((self.owner.clone(), id.clone()), (at, seq));
                s.deadlines
                    .push(Reverse((at, seq, self.owner.clone(), id.clone())));
            }
        }
        compact(&mut s);
        shared.changed.notify_all();
        loop {
            if s.shutdown || owner_closed(&s, &self.owner) {
                return Err("Execution owner is closed".into());
            }
            if let Some(batch) = claim(&mut s, &self.owner) {
                return Ok(batch);
            }
            drop(s);
            check()?;
            s = shared
                .store
                .lock()
                .map_err(|_| "Execution manager unavailable")?;
            if let Some(batch) = claim(&mut s, &self.owner) {
                return Ok(batch);
            }
            s = shared
                .changed
                .wait_timeout(s, Duration::from_millis(50))
                .map_err(|_| "Execution manager unavailable")?
                .0;
        }
    }
    pub fn read(
        &self,
        id: &str,
        tail: Option<usize>,
        cursor: Option<u64>,
    ) -> Result<ExecutionRead, String> {
        let mut s = self
            .manager
            .0
            .shared
            .store
            .lock()
            .map_err(|_| "Execution manager unavailable")?;
        owned(&s, &self.owner, id)?;
        let r = s.records.get_mut(id).unwrap();
        let mut output = r.output.read(tail, cursor)?;
        output.complete_result &= terminal(r.status);
        let format = if output.complete_result && r.structured {
            "json"
        } else {
            "text"
        };
        Ok(ExecutionRead {
            execution: info(id, r),
            output,
            metadata: r.metadata.clone(),
            error: r.error.clone(),
            format,
        })
    }
    pub(crate) fn peek_images(&self, id: &str) -> Result<Vec<ToolAttachment>, String> {
        let s = self
            .manager
            .0
            .shared
            .store
            .lock()
            .map_err(|_| "Execution manager unavailable")?;
        Ok(owned(&s, &self.owner, id)?.attachments.clone())
    }
    pub(crate) fn take_images(&self, id: &str) -> Result<Vec<ToolAttachment>, String> {
        let mut s = self
            .manager
            .0
            .shared
            .store
            .lock()
            .map_err(|_| "Execution manager unavailable")?;
        owned(&s, &self.owner, id)?;
        Ok(std::mem::take(
            &mut s.records.get_mut(id).unwrap().attachments,
        ))
    }
    pub fn cancel(&self, id: &str) -> Result<ExecutionInfo, String> {
        let mut s = self
            .manager
            .0
            .shared
            .store
            .lock()
            .map_err(|_| "Execution manager unavailable")?;
        owned(&s, &self.owner, id)?;
        let r = s.records.get_mut(id).unwrap();
        if !terminal(r.status) {
            r.cancel_requested = true;
            r.status = "cancelling";
            r.revision += 1;
        }
        let result = info(id, r);
        self.manager.0.shared.changed.notify_all();
        Ok(result)
    }
}
fn owned<'a>(s: &'a Store, owner: &str, id: &str) -> Result<&'a Record, String> {
    if owner_closed(s, owner) {
        return Err("Execution owner is closed".into());
    }
    s.records
        .get(id)
        .filter(|r| r.owner == owner)
        .ok_or("Execution not found in this agent scope".into())
}
impl ExecutionReporter {
    pub fn cancelled(&self) -> bool {
        let s = self.shared.store.lock().unwrap_or_else(|e| e.into_inner());
        s.shutdown
            || s.records
                .get(&self.id)
                .is_none_or(|r| r.cancel_requested || owner_closed(&s, &r.owner))
    }
    pub fn ack_cancel(&self) {
        if let Some(r) = self
            .shared
            .store
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .records
            .get_mut(&self.id)
        {
            r.cancel_ack = true;
        }
    }
    pub fn append(&self, text: &str) {
        let mut s = self.shared.store.lock().unwrap_or_else(|e| e.into_inner());
        if s.records
            .get(&self.id)
            .is_some_and(|r| owner_closed(&s, &r.owner))
        {
            return;
        }
        if let Some(r) = s.records.get_mut(&self.id) {
            if !terminal(r.status) {
                r.output.append(text);
                r.revision += 1;
            }
        }
    }
    pub fn metadata(&self, value: serde_json::Value) {
        if !value.is_object() {
            return;
        }
        let encoded = value.to_string();
        if encoded.len() > 4096 {
            return;
        }
        let mut s = self.shared.store.lock().unwrap_or_else(|e| e.into_inner());
        if s.records
            .get(&self.id)
            .is_some_and(|r| owner_closed(&s, &r.owner))
        {
            return;
        }
        if let Some(r) = s.records.get_mut(&self.id) {
            if !terminal(r.status) {
                r.metadata = value;
                r.revision += 1;
            }
        }
    }
    pub fn input_required(&self) {
        let mut s = self.shared.store.lock().unwrap_or_else(|e| e.into_inner());
        if s.records.get(&self.id).is_some_and(|r| !terminal(r.status)) {
            enqueue(&mut s, &self.id, "input_required");
            self.shared.changed.notify_all();
        }
    }
    fn finish(&self, result: Result<ToolResult, String>) {
        let mut s = self.shared.store.lock().unwrap_or_else(|e| e.into_inner());
        let image_bytes: usize = s
            .records
            .values()
            .flat_map(|r| &r.attachments)
            .map(|a| match a {
                ToolAttachment::Image(i) => i.bytes.len(),
            })
            .sum();
        let closed = s
            .records
            .get(&self.id)
            .is_some_and(|r| owner_closed(&s, &r.owner));
        let images_allowed = s.records.get(&self.id).is_some_and(|r| {
            s.mailboxes
                .get(&r.owner)
                .is_some_and(|b| b.consumer && b.image_epoch == r.image_epoch)
        });
        let Some(r) = s.records.get_mut(&self.id) else {
            return;
        };
        if closed {
            r.status = if r.cancel_ack {
                "cancelled"
            } else if result.is_ok() {
                "succeeded"
            } else {
                "failed"
            };
            r.output = output::Output::default();
            r.attachments.clear();
            r.metadata = serde_json::json!({});
            r.error = None;
            r.revision += 1;
            self.shared.changed.notify_all();
            return;
        }
        if terminal(r.status) {
            return;
        }
        r.revision += 1;
        match result {
            Ok(result) => {
                if r.output.end() == 0 {
                    r.structured = serde_json::from_str::<serde_json::Value>(&result.message)
                        .is_ok_and(|v| v.is_object() || v.is_array());
                    r.output.append(&result.message);
                }
                let bytes: usize = result
                    .attachments
                    .iter()
                    .map(|a| match a {
                        ToolAttachment::Image(i) => i.bytes.len(),
                    })
                    .sum();
                if bytes > 0 && !images_allowed {
                    r.status = "failed";
                    r.error=Some("Image result expired with its requesting turn; capture again with approval if needed".into());
                } else if image_bytes.saturating_add(bytes) > 32 * 1024 * 1024 {
                    r.status = "failed";
                    r.error = Some(
                        "Asynchronous image retention exceeds32MiB; use direct image observation"
                            .into(),
                    );
                } else {
                    r.attachments = result.attachments;
                    r.status = if r.cancel_ack
                        || r.metadata.get("stopped") == Some(&serde_json::Value::Bool(true))
                    {
                        "cancelled"
                    } else if r
                        .metadata
                        .get("status")
                        .and_then(|v| v.as_str())
                        .is_some_and(|status| matches!(status, "failed" | "timedOut" | "timed_out"))
                        || r.metadata
                            .get("exit_code")
                            .and_then(|v| v.as_i64())
                            .is_some_and(|v| v != 0)
                        || r.metadata.get("timed_out") == Some(&serde_json::Value::Bool(true))
                        || r.metadata.get("error").is_some_and(|v| !v.is_null())
                    {
                        "failed"
                    } else {
                        "succeeded"
                    };
                }
            }
            Err(error) => {
                r.status = if r.cancel_ack { "cancelled" } else { "failed" };
                if r.output.end() > 0 {
                    r.output.append("\n[error]\n");
                }
                r.output.append(&error);
                r.structured = false;
                r.error =
                    Some("Tool execution failed; read selected output lines for details".into());
            }
        }
        let reason = if r.status == "failed" {
            "failed"
        } else if r.status == "cancelled" {
            "cancelled"
        } else {
            "completed"
        };
        let owner = r.owner.clone();
        let fields = serde_json::json!({"executionId":self.id,"tool":r.tool,"status":r.status,"durationMs":elapsed(r.started)});
        let success = r.status == "succeeded";
        s.schedules.remove(&(owner, self.id.clone()));
        enqueue(&mut s, &self.id, reason);
        self.shared.changed.notify_all();
        drop(s);
        crate::diagnostics::event(
            if success { "INFO" } else { "WARN" },
            "execution.finished",
            fields,
        );
    }
}
fn compact(s: &mut Store) {
    if s.deadlines.len() > s.schedules.len() * 2 + 128 {
        s.deadlines = s
            .schedules
            .iter()
            .map(|((owner, id), (at, seq))| Reverse((*at, *seq, owner.clone(), id.clone())))
            .collect();
    }
}
fn scheduler_loop(shared: Arc<Shared>) {
    let mut s = shared.store.lock().unwrap_or_else(|e| e.into_inner());
    loop {
        if s.shutdown {
            return;
        }
        let now = Instant::now();
        let expired = s
            .owner_deadlines
            .iter()
            .filter(|(_, at)| **at <= now)
            .map(|(owner, _)| owner.clone())
            .collect::<Vec<_>>();
        for owner in expired {
            close_store_owner(&mut s, &owner);
            shared.changed.notify_all();
        }
        while let Some(Reverse((at, seq, owner, id))) = s.deadlines.peek().cloned() {
            if at > now {
                break;
            }
            s.deadlines.pop();
            if s.schedules.get(&(owner.clone(), id.clone())) != Some(&(at, seq)) {
                continue;
            }
            s.schedules.remove(&(owner, id.clone()));
            if s.records.get(&id).is_some_and(|r| !terminal(r.status)) {
                enqueue(&mut s, &id, "interval_elapsed");
                shared.changed.notify_all();
            }
        }
        let owner_wait = s
            .owner_deadlines
            .values()
            .min()
            .map(|at| at.saturating_duration_since(Instant::now()))
            .unwrap_or(Duration::from_secs(30));
        let wait = s
            .deadlines
            .peek()
            .map(|Reverse((at, ..))| at.saturating_duration_since(Instant::now()))
            .unwrap_or(Duration::from_secs(30))
            .min(owner_wait);
        s = shared
            .changed
            .wait_timeout(s, wait)
            .unwrap_or_else(|e| e.into_inner())
            .0;
    }
}
impl Drop for ConsumerLease {
    fn drop(&mut self) {
        let mut s = self.shared.store.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(b) = s.mailboxes.get_mut(&self.owner) {
            b.consumer = false;
        }
        for r in s.records.values_mut().filter(|r| r.owner == self.owner) {
            if !r.attachments.is_empty() {
                r.attachments.clear();
                r.metadata["image_expired"] = serde_json::json!(true);
            }
        }
    }
}
impl Drop for Coordinator {
    fn drop(&mut self) {
        self.shared
            .store
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .shutdown = true;
        self.shared.changed.notify_all();
        if let Some(worker) = self
            .scheduler
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take()
        {
            let _ = worker.join();
        }
    }
}

#[cfg(test)]
mod privacy_tests {
    use super::*;
    fn image() -> ToolResult {
        ToolResult::with_image(
            "caption",
            crate::media::ImageData {
                mime_type: "image/png".into(),
                bytes: vec![1, 2, 3],
                width: 1,
                height: 1,
                detail: crate::media::ImageDetail::Low,
            },
        )
    }
    #[test]
    fn closed_scopes_discard_existing_and_late_payloads() {
        let manager = ToolExecutionManager::new();
        let scope = manager.scope("private");
        let _lease = scope.acquire_consumer().unwrap();
        let (ready, started) = std::sync::mpsc::channel();
        let (release, gate) = std::sync::mpsc::channel();
        let info = scope
            .start("fake", move |reporter| {
                reporter.append("before closure");
                ready.send(()).unwrap();
                gate.recv().unwrap();
                reporter.append("late private log");
                reporter.metadata(serde_json::json!({"late":"private"}));
                Ok(image())
            })
            .unwrap();
        started.recv().unwrap();
        scope.close();
        release.send(()).unwrap();
        let deadline = Instant::now() + Duration::from_secs(1);
        loop {
            let s = manager.0.shared.store.lock().unwrap();
            let r = &s.records[&info.execution_id];
            if terminal(r.status) {
                assert!(r.attachments.is_empty());
                assert_eq!(r.output.end(), 0);
                assert_eq!(r.metadata, serde_json::json!({}));
                break;
            }
            assert!(Instant::now() < deadline);
            drop(s);
            thread::sleep(Duration::from_millis(2));
        }
    }
    #[test]
    fn images_expire_with_the_request_even_if_a_new_consumer_has_started() {
        let manager = ToolExecutionManager::new();
        let scope = manager.scope("owner");
        let lease = scope.acquire_consumer().unwrap();
        let info = scope.start("fake", |_| Ok(image())).unwrap();
        let batch = scope
            .wait(std::slice::from_ref(&info.execution_id), 100)
            .unwrap();
        scope.ack(&batch.batch_id).unwrap();
        assert!(scope.info(&info.execution_id).unwrap().has_image);
        drop(lease);
        assert!(!scope.info(&info.execution_id).unwrap().has_image);
        let lease = scope.acquire_consumer().unwrap();
        let (release, gate) = std::sync::mpsc::channel();
        let late = scope
            .start("fake", move |_| {
                gate.recv().unwrap();
                Ok(image())
            })
            .unwrap();
        drop(lease);
        let _new_request = scope.acquire_consumer().unwrap();
        release.send(()).unwrap();
        let batch = scope
            .wait(std::slice::from_ref(&late.execution_id), 100)
            .unwrap();
        assert_eq!(batch.events[0].status, "failed");
        assert!(!scope.info(&late.execution_id).unwrap().has_image);
    }
}
