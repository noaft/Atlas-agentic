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
    newlines: u64,
    ends_with_newline: bool,
    base_mid_line: bool,
}
impl Output {
    fn append(&mut self, bytes: &[u8], limit: usize) {
        self.end = self.end.saturating_add(bytes.len() as u64);
        let discard = self
            .bytes
            .len()
            .saturating_add(bytes.len())
            .saturating_sub(limit);
        if discard > 0 {
            let last_discarded = if discard > self.bytes.len() {
                bytes[discard - self.bytes.len() - 1]
            } else {
                self.bytes[discard - 1]
            };
            self.base_mid_line = last_discarded != b'\n';
        }
        self.newlines = self
            .newlines
            .saturating_add(bytes.iter().filter(|b| **b == b'\n').count() as u64);
        if let Some(last) = bytes.last() {
            self.ends_with_newline = *last == b'\n';
        }
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
    fn tail_start(&mut self, lines: usize, done: bool) -> u64 {
        let bytes = self.bytes.make_contiguous();
        let end = decoded_len(bytes, done);
        let mut remaining = lines;
        for index in (0..end).rev() {
            if bytes[index] == b'\n' && index + 1 < end {
                remaining -= 1;
                if remaining == 0 {
                    return self.base + index as u64 + 1;
                }
            }
        }
        self.base
    }
    fn position(&mut self, requested_start: u64, end_byte: u64) -> ProcessOutputView {
        let start_byte = requested_start.max(self.base).min(self.end);
        let bytes = self.bytes.make_contiguous();
        let start = (start_byte - self.base) as usize;
        let end = (end_byte - self.base) as usize;
        let retained_newlines = bytes.iter().filter(|b| **b == b'\n').count() as u64;
        let total_lines = self.newlines + u64::from(self.end > 0 && !self.ends_with_newline);
        let start_line = (end > start).then(|| {
            self.newlines - retained_newlines
                + 1
                + bytes[..start].iter().filter(|b| **b == b'\n').count() as u64
        });
        let end_line = start_line.map(|line| {
            line + bytes[start..end].iter().filter(|b| **b == b'\n').count() as u64
                - u64::from(bytes[end - 1] == b'\n')
        });
        ProcessOutputView {
            start_byte,
            end_byte,
            total_bytes: self.end,
            start_line,
            end_line,
            total_lines,
            returned_lines: start_line
                .zip(end_line)
                .map_or(0, |(start, end)| end - start + 1),
            lines_after: end_line.map(|line| total_lines.saturating_sub(line)),
            starts_mid_line: end > start
                && if start == 0 {
                    self.base_mid_line
                } else {
                    bytes[start - 1] != b'\n'
                },
            older_output_dropped: self.base > 0,
            has_more: end_byte < self.end,
        }
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
pub struct ProcessOutputView {
    pub start_byte: u64,
    pub end_byte: u64,
    pub total_bytes: u64,
    pub start_line: Option<u64>,
    pub end_line: Option<u64>,
    pub total_lines: u64,
    pub returned_lines: u64,
    pub lines_after: Option<u64>,
    pub starts_mid_line: bool,
    pub older_output_dropped: bool,
    pub has_more: bool,
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
    pub tail_lines: Option<usize>,
    pub stdout_view: ProcessOutputView,
    pub stderr_view: ProcessOutputView,
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
        self.snapshot_selection(id, cursor, None, wait, until_done)
    }
    pub(super) fn tail_snapshot(
        &self,
        id: &str,
        lines: usize,
        wait: Duration,
    ) -> Result<ProcessRead, String> {
        self.snapshot_selection(id, None, Some(lines), wait, false)
    }
    fn snapshot_selection(
        &self,
        id: &str,
        cursor: Option<ProcessCursor>,
        tail_lines: Option<usize>,
        wait: Duration,
        until_done: bool,
    ) -> Result<ProcessRead, String> {
        let deadline = Instant::now() + wait;
        let mut p = self.progress.lock().unwrap_or_else(|e| e.into_inner());
        let cursor = if tail_lines.is_some() {
            ProcessCursor {
                stdout: p.stdout.base,
                stderr: p.stderr.base,
            }
        } else {
            cursor.unwrap_or(p.agent)
        };
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
        let cursor = if let Some(lines) = tail_lines {
            ProcessCursor {
                stdout: p.stdout.tail_start(lines, done),
                stderr: p.stderr.tail_start(lines, done),
            }
        } else {
            cursor
        };
        let (stdout, out, out_gap) = p.stdout.read(cursor.stdout, done);
        let (stderr, err, err_gap) = p.stderr.read(cursor.stderr, done);
        let stdout_view = p.stdout.position(cursor.stdout, out);
        let stderr_view = p.stderr.position(cursor.stderr, err);
        let next_cursor = ProcessCursor {
            stdout: out,
            stderr: err,
        };
        Ok(ProcessRead {
            session_id: id.into(),
            running: !done,
            stdout,
            stderr,
            tail_lines,
            stdout_view,
            stderr_view,
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

#[cfg(test)]
mod position_tests {
    use super::*;

    #[test]
    fn reports_absolute_lines_and_partial_oldest_line_after_overflow() {
        let control = ProcessControl::new(8);
        control.capture(false, b"one\ntwo\nthree\n");
        let result = control
            .snapshot("id", Some(ProcessCursor::default()), Duration::ZERO, false)
            .unwrap();
        assert_eq!(result.stdout, "o\nthree\n");
        let value = serde_json::to_value(result).unwrap();
        assert_eq!(value["stdout_view"]["start_byte"], 6);
        assert_eq!(value["stdout_view"]["end_byte"], 14);
        assert_eq!(value["stdout_view"]["start_line"], 2);
        assert_eq!(value["stdout_view"]["end_line"], 3);
        assert_eq!(value["stdout_view"]["total_lines"], 3);
        assert_eq!(value["stdout_view"]["returned_lines"], 2);
        assert_eq!(value["stdout_view"]["older_output_dropped"], true);
        assert_eq!(value["stdout_view"]["starts_mid_line"], true);
    }

    #[test]
    fn incomplete_unicode_has_pending_bytes_and_keeps_line_positions() {
        let control = ProcessControl::new(16);
        control.capture(false, b"one\n\xe1\xbb");
        let result = control.read_agent("id", Duration::ZERO, false).unwrap();
        assert_eq!(result.stdout, "one\n");
        let value = serde_json::to_value(result).unwrap();
        assert_eq!(value["stdout_view"]["end_line"], 1);
        assert_eq!(value["stdout_view"]["lines_after"], 1);
        assert_eq!(value["stdout_view"]["has_more"], true);
        control.capture(false, &[0x87]);
        let result = control.read_agent("id", Duration::ZERO, false).unwrap();
        assert_eq!(result.stdout, "ệ");
        let value = serde_json::to_value(result).unwrap();
        assert_eq!(value["stdout_view"]["start_line"], 2);
        assert_eq!(value["stdout_view"]["has_more"], false);
    }
}

#[cfg(test)]
mod tail_tests {
    use super::*;

    #[test]
    fn tail_counts_lf_lines_without_inventing_a_trailing_empty_line() {
        for (bytes, count, expected, start, end) in [
            ("one\r\ntwo\r\n", 1, "two\r\n", Some(2), Some(2)),
            ("one\ntwo", 1, "two", Some(2), Some(2)),
            ("\n\n", 1, "\n", Some(2), Some(2)),
            ("one", 50, "one", Some(1), Some(1)),
            ("", 1, "", None, None),
        ] {
            let control = ProcessControl::new(64);
            control.capture(false, bytes.as_bytes());
            let read = control.tail_snapshot("id", count, Duration::ZERO).unwrap();
            assert_eq!(read.stdout, expected);
            assert_eq!(read.stdout_view.start_line, start);
            assert_eq!(read.stdout_view.end_line, end);
            assert!(!read.stdout_truncated);
        }
    }

    #[test]
    fn repeated_overflow_keeps_absolute_lines_and_tail_skips_partial_oldest_line() {
        let control = ProcessControl::new(8);
        control.capture(false, b"a\nbb\n");
        control.capture(false, b"cc\n");
        control.capture(false, b"dd\n");
        let read = control.tail_snapshot("id", 2, Duration::ZERO).unwrap();
        assert_eq!(read.stdout, "cc\ndd\n");
        assert_eq!(read.stdout_view.start_byte, 5);
        assert_eq!(read.stdout_view.total_bytes, 11);
        assert_eq!(read.stdout_view.start_line, Some(3));
        assert_eq!(read.stdout_view.end_line, Some(4));
        assert_eq!(read.stdout_view.total_lines, 4);
        assert!(read.stdout_view.older_output_dropped);
        assert!(!read.stdout_view.starts_mid_line);
        assert!(!read.stdout_truncated);
    }

    #[test]
    fn tail_holds_incomplete_unicode_and_does_not_advance_the_default_reader() {
        let control = ProcessControl::new(64);
        control.capture(false, b"one\n\xe1\xbb");
        let tail = control.tail_snapshot("id", 1, Duration::ZERO).unwrap();
        assert_eq!(tail.stdout, "one\n");
        assert!(tail.stdout_view.has_more);
        control.capture(false, &[0x87]);
        let tail = control.tail_snapshot("id", 1, Duration::ZERO).unwrap();
        assert_eq!(tail.stdout, "ệ");
        assert_eq!(tail.stdout_view.start_line, Some(2));
        assert_eq!(
            control
                .read_agent("id", Duration::ZERO, false)
                .unwrap()
                .stdout,
            "one\nệ"
        );
    }

    #[test]
    fn empty_tail_waits_for_output_with_the_existing_bounded_deadline() {
        let control = std::sync::Arc::new(ProcessControl::new(64));
        let other = control.clone();
        let worker = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(20));
            other.capture(true, b"first\nlast\n");
        });
        let read = control
            .tail_snapshot("id", 1, Duration::from_millis(500))
            .unwrap();
        assert_eq!(read.stderr, "last\n");
        worker.join().unwrap();
    }
}
