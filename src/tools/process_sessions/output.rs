//! Bounded replayable output with independent readers and byte offsets.
use super::ProcessCursor;
use serde::{Deserialize, Serialize};
use std::{
    collections::VecDeque,
    sync::{
        atomic::{AtomicBool, Ordering},
        Condvar, Mutex,
    },
    time::{Duration, Instant},
};
#[derive(Default)]
struct Output {
    bytes: VecDeque<u8>,
    base: u64,
    end: u64,
}
impl Output {
    fn append(&mut self, bytes: &[u8], limit: usize) {
        self.end = self.end.saturating_add(bytes.len() as u64);
        let discard = self
            .bytes
            .len()
            .saturating_add(bytes.len())
            .saturating_sub(limit);
        self.bytes.drain(..discard.min(self.bytes.len()));
        self.bytes
            .extend(bytes.iter().skip(bytes.len().saturating_sub(limit)));
        self.base = self.end - self.bytes.len() as u64;
    }
    fn read(&mut self, cursor: u64, done: bool) -> (String, u64, bool) {
        let gap = cursor < self.base;
        let cursor = cursor.max(self.base).min(self.end);
        let bytes = self.bytes.make_contiguous();
        let offset = (cursor - self.base) as usize;
        let count = decoded_len(&bytes[offset..], done);
        (
            String::from_utf8_lossy(&bytes[offset..offset + count]).into_owned(),
            cursor + count as u64,
            gap,
        )
    }
    fn ready(&mut self, cursor: u64) -> bool {
        let (text, _, gap) = self.read(cursor, false);
        gap || !text.is_empty()
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
#[derive(Default, Deserialize)]
pub(super) struct Completion {
    pub exit_code: Option<i32>,
    #[serde(default)]
    pub timed_out: bool,
    #[serde(default)]
    pub stopped: bool,
    #[serde(default)]
    pub error: Option<String>,
}
#[derive(Default)]
struct Progress {
    stdout: Output,
    stderr: Output,
    agent: ProcessCursor,
    started: bool,
    done: Option<Completion>,
}
pub(crate) struct ProcessControl {
    cancelled: AtomicBool,
    progress: Mutex<Progress>,
    changed: Condvar,
    limit: usize,
    agent_reader: Mutex<()>,
}
#[derive(Debug, Clone, Serialize)]
pub struct ProcessRead {
    pub session_id: String,
    pub running: bool,
    pub stdout: String,
    pub stderr: String,
    pub stdout_truncated: bool,
    pub stderr_truncated: bool,
    pub next_cursor: ProcessCursor,
    pub oldest_cursor: ProcessCursor,
    pub exit_code: Option<i32>,
    pub timed_out: bool,
    pub stopped: bool,
    pub stop_requested: bool,
    pub error: Option<String>,
}
impl ProcessControl {
    pub(super) fn new(limit: usize) -> Self {
        Self {
            cancelled: AtomicBool::new(false),
            progress: Mutex::new(Progress::default()),
            changed: Condvar::new(),
            limit: limit.min(super::OUTPUT_LIMIT),
            agent_reader: Mutex::new(()),
        }
    }
    pub(crate) fn stopped(&self) -> bool {
        self.cancelled.load(Ordering::SeqCst)
    }
    pub(super) fn cancel(&self) {
        self.cancelled.store(true, Ordering::SeqCst);
        self.changed.notify_all();
    }
    pub(crate) fn mark_running(&self) {
        self.progress
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .started = true;
        self.changed.notify_all();
    }
    pub(crate) fn capture(&self, stderr: bool, bytes: &[u8]) {
        let mut p = self.progress.lock().unwrap_or_else(|e| e.into_inner());
        if stderr {
            p.stderr.append(bytes, self.limit)
        } else {
            p.stdout.append(bytes, self.limit)
        }
        self.changed.notify_all();
    }
    pub(super) fn complete(&self, completion: Completion) {
        self.progress.lock().unwrap_or_else(|e| e.into_inner()).done = Some(completion);
        self.changed.notify_all();
    }
    pub(super) fn is_done(&self) -> bool {
        self.progress
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .done
            .is_some()
    }
    pub(super) fn state(&self) -> (&'static str, bool, Option<i32>) {
        let p = self.progress.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(done) = &p.done {
            (
                if done.timed_out {
                    "timedOut"
                } else if done.stopped {
                    "stopped"
                } else if done.error.is_some() || done.exit_code != Some(0) {
                    "failed"
                } else {
                    "exited"
                },
                false,
                done.exit_code,
            )
        } else {
            (
                if self.stopped() {
                    "stopping"
                } else if p.started {
                    "running"
                } else {
                    "starting"
                },
                true,
                None,
            )
        }
    }
    pub(super) fn snapshot(
        &self,
        id: &str,
        cursor: Option<ProcessCursor>,
        wait: Duration,
        until_done: bool,
    ) -> Result<ProcessRead, String> {
        let deadline = Instant::now() + wait;
        let mut p = self.progress.lock().unwrap_or_else(|e| e.into_inner());
        let cursor = cursor.unwrap_or(p.agent);
        if cursor.stdout > p.stdout.end || cursor.stderr > p.stderr.end {
            return Err("Process cursor is ahead of available output".into());
        }
        loop {
            if p.done.is_some()
                || (!until_done && (p.stdout.ready(cursor.stdout) || p.stderr.ready(cursor.stderr)))
                || Instant::now() >= deadline
            {
                break;
            }
            p = self
                .changed
                .wait_timeout(p, deadline.saturating_duration_since(Instant::now()))
                .unwrap_or_else(|e| e.into_inner())
                .0;
        }
        let done = p.done.is_some();
        let (stdout, out, out_gap) = p.stdout.read(cursor.stdout, done);
        let (stderr, err, err_gap) = p.stderr.read(cursor.stderr, done);
        let next_cursor = ProcessCursor {
            stdout: out,
            stderr: err,
        };
        Ok(ProcessRead {
            session_id: id.into(),
            running: !done,
            stdout,
            stderr,
            stdout_truncated: out_gap,
            stderr_truncated: err_gap,
            next_cursor,
            oldest_cursor: ProcessCursor {
                stdout: p.stdout.base,
                stderr: p.stderr.base,
            },
            exit_code: p.done.as_ref().and_then(|d| d.exit_code),
            timed_out: p.done.as_ref().is_some_and(|d| d.timed_out),
            stopped: p.done.as_ref().is_some_and(|d| d.stopped),
            stop_requested: self.stopped(),
            error: p.done.as_ref().and_then(|d| d.error.clone()),
        })
    }
    pub(super) fn read_agent(
        &self,
        id: &str,
        wait: Duration,
        until_done: bool,
    ) -> Result<ProcessRead, String> {
        let _reader = self
            .agent_reader
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let result = self.snapshot(id, None, wait, until_done)?;
        self.progress
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .agent = result.next_cursor;
        Ok(result)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn invalid_prefix_and_split_unicode_survive_replay() {
        let control = ProcessControl::new(8);
        control.capture(false, &[255, 0xe1, 0xbb]);
        let first = control.read_agent("id", Duration::ZERO, false).unwrap();
        assert_eq!(first.stdout, "�");
        control.capture(false, &[0x87]);
        assert_eq!(
            control
                .read_agent("id", Duration::ZERO, false)
                .unwrap()
                .stdout,
            "ệ"
        );
        assert_eq!(
            control
                .snapshot("id", Some(ProcessCursor::default()), Duration::ZERO, false)
                .unwrap()
                .stdout,
            "�ệ"
        );
    }
    #[test]
    fn overflow_reports_gaps_per_reader() {
        let c = ProcessControl::new(8);
        c.capture(false, b"1234567890");
        let read = c
            .snapshot("id", Some(ProcessCursor::default()), Duration::ZERO, false)
            .unwrap();
        assert_eq!(read.stdout, "34567890");
        assert!(read.stdout_truncated);
        assert!(
            !c.snapshot("id", Some(read.next_cursor), Duration::ZERO, false)
                .unwrap()
                .stdout_truncated
        );
    }
    #[test]
    fn incomplete_character_waits_for_continuation() {
        let c = std::sync::Arc::new(ProcessControl::new(8));
        c.capture(false, &[0xe1, 0xbb]);
        let other = c.clone();
        let worker = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(20));
            other.capture(false, &[0x87]);
        });
        assert_eq!(
            c.read_agent("id", Duration::from_millis(500), false)
                .unwrap()
                .stdout,
            "ệ"
        );
        worker.join().unwrap();
    }
}
