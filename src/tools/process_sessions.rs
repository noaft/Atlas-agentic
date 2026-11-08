//! Bounded, request-owned shell sessions. Workers retain existing platform cleanup.
use super::{ExecCommandTool, Tool, ToolDefinition, ToolRegistry, ToolResult};
use crate::permissions::ToolRisk;
use serde::{Deserialize, Serialize};
use std::{
    collections::{HashMap, VecDeque},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Condvar, Mutex,
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};
const MAX_WAIT_MS: u64 = 10_000;
const MAX_TIMEOUT_MS: u64 = 600_000;
const MAX_ACTIVE: usize = 8;
const MAX_RETAINED: usize = 32;
const OUTPUT_LIMIT: usize = 64 * 1024;
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ExecArgs {
    pub command: String,
    pub yield_time_ms: Option<u64>,
    pub timeout_ms: Option<u64>,
}
pub(super) fn parse_exec(args: &serde_json::Value) -> Result<ExecArgs, String> {
    let args: ExecArgs =
        serde_json::from_value(args.clone()).map_err(|_| "Invalid exec_command arguments")?;
    if args.command.trim().is_empty() {
        return Err("exec_command command is required".into());
    }
    if args.yield_time_ms.is_some_and(|ms| ms > MAX_WAIT_MS)
        || args
            .timeout_ms
            .is_some_and(|ms| ms == 0 || ms > MAX_TIMEOUT_MS)
    {
        return Err("yield_time_ms must be 0–10000 and timeout_ms must be 1–600000".into());
    }
    Ok(args)
}
pub(super) fn exec_definition() -> ToolDefinition {
    ToolDefinition{name:"exec_command".into(),description:"Execute an unrestricted shell command from the workspace. Shell operators and outside paths are allowed. Default: synchronous, 30-second timeout. Set yield_time_ms (0–10000) to return a request-owned session_id when still running; read_process polls it and stop_process stops it. Run session jobs in the foreground; remaining contained children stop when the shell finishes. Session timeout defaults to 10 minutes. All sessions are terminated when this agent request ends; they do not survive to the next message.".into(),input_schema:serde_json::json!({"type":"object","required":["command"],"additionalProperties":false,"properties":{"command":{"type":"string"},"yield_time_ms":{"type":"integer","minimum":0,"maximum":10000},"timeout_ms":{"type":"integer","minimum":1,"maximum":600000}}})}
}
#[derive(Default)]
struct Output {
    bytes: VecDeque<u8>,
    truncated: bool,
}
impl Output {
    fn append(&mut self, bytes: &[u8], limit: usize) {
        let discard = self
            .bytes
            .len()
            .saturating_add(bytes.len())
            .saturating_sub(limit);
        if discard > 0 {
            self.truncated = true;
            self.bytes.drain(..discard.min(self.bytes.len()));
        }
        self.bytes
            .extend(bytes.iter().skip(bytes.len().saturating_sub(limit)));
    }
    fn ready(&mut self) -> bool {
        self.truncated || decoded_len(self.bytes.make_contiguous(), false) > 0
    }
    fn take(&mut self, done: bool) -> (String, bool) {
        let bytes = self.bytes.make_contiguous();
        let len = decoded_len(bytes, done);
        let text = String::from_utf8_lossy(&bytes[..len]).into_owned();
        self.bytes.drain(..len);
        (text, std::mem::take(&mut self.truncated))
    }
}
fn decoded_len(bytes: &[u8], done: bool) -> usize {
    if done {
        return bytes.len();
    }
    let mut offset = 0;
    loop {
        match std::str::from_utf8(&bytes[offset..]) {
            Ok(_) => return bytes.len(),
            Err(error) => {
                offset += error.valid_up_to();
                match error.error_len() {
                    Some(invalid) => offset += invalid,
                    None => return offset,
                }
            }
        }
    }
}

#[derive(Default, Deserialize, Serialize)]
struct Completion {
    exit_code: Option<i32>,
    #[serde(default)]
    timed_out: bool,
    #[serde(default)]
    stopped: bool,
    #[serde(default)]
    error: Option<String>,
}
#[derive(Default)]
struct Progress {
    stdout: Output,
    stderr: Output,
    done: Option<Completion>,
}
pub(super) struct ProcessControl {
    cancelled: AtomicBool,
    progress: Mutex<Progress>,
    changed: Condvar,
    limit: usize,
}
impl ProcessControl {
    fn new(limit: usize) -> Self {
        Self {
            cancelled: AtomicBool::new(false),
            progress: Mutex::new(Progress::default()),
            changed: Condvar::new(),
            limit: limit.min(OUTPUT_LIMIT),
        }
    }
    pub(super) fn stopped(&self) -> bool {
        self.cancelled.load(Ordering::SeqCst)
    }
    pub(super) fn capture(&self, stderr: bool, bytes: &[u8]) {
        let mut progress = self.progress.lock().unwrap_or_else(|e| e.into_inner());
        if stderr {
            progress.stderr.append(bytes, self.limit);
        } else {
            progress.stdout.append(bytes, self.limit);
        }
        self.changed.notify_all();
    }
    fn complete(&self, completion: Completion) {
        self.progress.lock().unwrap_or_else(|e| e.into_inner()).done = Some(completion);
        self.changed.notify_all();
    }
    fn is_done(&self) -> bool {
        self.progress
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .done
            .is_some()
    }
    fn snapshot(&self, id: &str, wait: Duration, until_done: bool) -> ToolResult {
        let deadline = Instant::now() + wait;
        let mut progress = self.progress.lock().unwrap_or_else(|e| e.into_inner());
        loop {
            if progress.done.is_some()
                || (!until_done && (progress.stdout.ready() || progress.stderr.ready()))
                || Instant::now() >= deadline
            {
                break;
            }
            progress = self
                .changed
                .wait_timeout(progress, deadline.saturating_duration_since(Instant::now()))
                .unwrap_or_else(|e| e.into_inner())
                .0;
        }
        let done = progress.done.is_some();
        let (stdout, out_truncated) = progress.stdout.take(done);
        let (stderr, err_truncated) = progress.stderr.take(done);
        let result = serde_json::json!({"session_id":id,"running":!done,"stdout":stdout,"stderr":stderr,"stdout_truncated":out_truncated,"stderr_truncated":err_truncated,"exit_code":progress.done.as_ref().and_then(|v|v.exit_code),"timed_out":progress.done.as_ref().is_some_and(|v|v.timed_out),"stopped":progress.done.as_ref().is_some_and(|v|v.stopped),"stop_requested":self.stopped(),"error":progress.done.as_ref().and_then(|v|v.error.as_deref())});
        ToolResult::text(result.to_string())
    }
}
struct Session {
    control: Arc<ProcessControl>,
    worker: Option<JoinHandle<()>>,
    created: Instant,
}
pub(super) struct ProcessSessions {
    workspace: PathBuf,
    output_limit: usize,
    sessions: Mutex<HashMap<String, Session>>,
}
impl ProcessSessions {
    pub(super) fn new(workspace: &Path, limit: usize) -> Self {
        Self {
            workspace: workspace.into(),
            output_limit: limit,
            sessions: Mutex::new(HashMap::new()),
        }
    }
    pub(super) fn start(&self, args: ExecArgs) -> Result<ToolResult, String> {
        let wait =
            Duration::from_millis(args.yield_time_ms.ok_or("Session yield time is required")?);
        let timeout = Duration::from_millis(args.timeout_ms.unwrap_or(MAX_TIMEOUT_MS));
        let id = uuid::Uuid::new_v4().to_string();
        let control = Arc::new(ProcessControl::new(self.output_limit));
        {
            let mut sessions = self
                .sessions
                .lock()
                .map_err(|_| "Process session registry unavailable")?;
            if sessions.values().filter(|s| !s.control.is_done()).count() >= MAX_ACTIVE {
                return Err(
                    "Eight process sessions are already running; stop or finish one first".into(),
                );
            }
            if sessions.len() >= MAX_RETAINED {
                if let Some(oldest) = sessions
                    .iter()
                    .filter(|(_, s)| s.control.is_done())
                    .min_by_key(|(_, s)| s.created)
                    .map(|(id, _)| id.clone())
                {
                    if let Some(mut old) = sessions.remove(&oldest) {
                        if let Some(worker) = old.worker.take() {
                            let _ = worker.join();
                        }
                    }
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
            sessions.insert(
                id.clone(),
                Session {
                    control: control.clone(),
                    worker: Some(worker),
                    created: Instant::now(),
                },
            );
        }
        Ok(control.snapshot(&id, wait, true))
    }
    fn get(&self, id: &str) -> Result<Arc<ProcessControl>, String> {
        self.sessions
            .lock()
            .map_err(|_| "Process session registry unavailable")?
            .get(id)
            .map(|s| s.control.clone())
            .ok_or("Unknown process session in this request".into())
    }
    pub(super) fn clear(&self) {
        let sessions =
            std::mem::take(&mut *self.sessions.lock().unwrap_or_else(|e| e.into_inner()));
        for session in sessions.values() {
            session.control.cancelled.store(true, Ordering::SeqCst);
        }
        for (_, mut session) in sessions {
            if let Some(worker) = session.worker.take() {
                let _ = worker.join();
            }
        }
    }
}
impl Drop for ProcessSessions {
    fn drop(&mut self) {
        self.clear();
    }
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PollArgs {
    session_id: String,
    #[serde(default = "default_wait")]
    wait_ms: u64,
}
fn default_wait() -> u64 {
    1000
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct StopArgs {
    session_id: String,
}
struct ProcessTool {
    sessions: Arc<ProcessSessions>,
    stop: bool,
}
impl ProcessTool {
    fn parse(&self, args: &serde_json::Value) -> Result<(String, u64), String> {
        let (id, wait) = if self.stop {
            let args: StopArgs = serde_json::from_value(args.clone())
                .map_err(|_| "Invalid stop_process arguments")?;
            (args.session_id, 2000)
        } else {
            let args: PollArgs = serde_json::from_value(args.clone())
                .map_err(|_| "Invalid read_process arguments")?;
            (args.session_id, args.wait_ms)
        };
        if wait > MAX_WAIT_MS {
            return Err("wait_ms must be 0–10000".into());
        }
        self.sessions.get(&id)?;
        Ok((id, wait))
    }
}
impl Tool for ProcessTool {
    fn name(&self) -> &str {
        if self.stop {
            "stop_process"
        } else {
            "read_process"
        }
    }
    fn definition(&self) -> ToolDefinition {
        let mut properties = serde_json::json!({"session_id":{"type":"string"}});
        if !self.stop {
            properties["wait_ms"] =
                serde_json::json!({"type":"integer","minimum":0,"maximum":10000,"default":1000});
        }
        ToolDefinition{name:self.name().into(),description:if self.stop{"Stop only the owned exec_command session and its contained descendants. Wait briefly for cleanup; running=true means stop is still pending. Stopping does not undo actions already performed."}else{"Read only new stdout/stderr and current status from an exec_command session in this request. Output is consumed, bounded to 64 KiB per stream, with explicit truncation. Session IDs do not survive the end of this agent request."}.into(),input_schema:serde_json::json!({"type":"object","required":["session_id"],"additionalProperties":false,"properties":properties})}
    }
    fn risk(&self, args: &serde_json::Value) -> Result<ToolRisk, String> {
        self.parse(args)?;
        Ok(if self.stop {
            ToolRisk::Command
        } else {
            ToolRisk::ReadOnly
        })
    }
    fn execute(&self, args: &serde_json::Value) -> Result<ToolResult, String> {
        let (id, wait) = self.parse(args)?;
        let control = self.sessions.get(&id)?;
        if self.stop {
            control.cancelled.store(true, Ordering::SeqCst);
        }
        Ok(control.snapshot(&id, Duration::from_millis(wait), self.stop))
    }
}
pub(super) fn session_tools(sessions: Arc<ProcessSessions>) -> Vec<Box<dyn Tool>> {
    [false, true]
        .into_iter()
        .map(|stop| {
            Box::new(ProcessTool {
                sessions: sessions.clone(),
                stop,
            }) as Box<dyn Tool>
        })
        .collect()
}
pub fn register_process_tools(registry: &mut ToolRegistry, workspace: &Path) {
    let exec = ExecCommandTool::new(workspace);
    for tool in exec.session_tools() {
        registry.register(tool);
    }
    registry.register(Box::new(exec));
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn completed_record_retention_is_bounded() {
        let root = tempfile::tempdir().unwrap();
        let sessions = ProcessSessions::new(root.path(), OUTPUT_LIMIT);
        for index in 0..MAX_RETAINED {
            let control = Arc::new(ProcessControl::new(8));
            control.complete(Completion::default());
            sessions.sessions.lock().unwrap().insert(
                format!("old-{index}"),
                Session {
                    control,
                    worker: None,
                    created: Instant::now(),
                },
            );
        }
        let result = sessions
            .start(ExecArgs {
                command: "echo done".into(),
                yield_time_ms: Some(1000),
                timeout_ms: None,
            })
            .unwrap();
        let result: serde_json::Value = serde_json::from_str(&result.message).unwrap();
        assert_eq!(sessions.sessions.lock().unwrap().len(), MAX_RETAINED);
        assert!(sessions.get(result["session_id"].as_str().unwrap()).is_ok());
    }
    #[test]
    fn invalid_prefix_does_not_consume_a_split_valid_suffix() {
        let control = ProcessControl::new(8);
        control.capture(false, &[255, 0xe1, 0xbb]);
        let first: serde_json::Value =
            serde_json::from_str(&control.snapshot("id", Duration::ZERO, false).message).unwrap();
        assert_eq!(first["stdout"], "�");
        control.capture(false, &[0x87]);
        let second: serde_json::Value =
            serde_json::from_str(&control.snapshot("id", Duration::ZERO, false).message).unwrap();
        assert_eq!(second["stdout"], "ệ");
    }
    #[test]
    fn poll_waits_for_a_complete_character() {
        let control = Arc::new(ProcessControl::new(8));
        control.capture(false, &[0xe1, 0xbb]);
        let producer = control.clone();
        let worker = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(20));
            producer.capture(false, &[0x87]);
        });
        let result: serde_json::Value = serde_json::from_str(
            &control
                .snapshot("id", Duration::from_millis(500), false)
                .message,
        )
        .unwrap();
        worker.join().unwrap();
        assert_eq!(result["stdout"], "ệ");
    }
    #[test]
    fn incremental_utf8_and_overflow_are_explicit() {
        let control = ProcessControl::new(8);
        control.capture(false, &[0xe1, 0xbb]);
        let result: serde_json::Value =
            serde_json::from_str(&control.snapshot("id", Duration::ZERO, false).message).unwrap();
        assert_eq!(result["stdout"], "");
        control.capture(false, &[0x87]);
        let result: serde_json::Value =
            serde_json::from_str(&control.snapshot("id", Duration::ZERO, false).message).unwrap();
        assert_eq!(result["stdout"], "ệ");
        control.capture(false, b"1234567890");
        let result: serde_json::Value =
            serde_json::from_str(&control.snapshot("id", Duration::ZERO, false).message).unwrap();
        assert_eq!(result["stdout"], "34567890");
        assert_eq!(result["stdout_truncated"], true);
    }
}
