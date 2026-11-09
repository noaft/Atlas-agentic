use std::{
    io::{self, Read},
    os::fd::AsRawFd,
    path::{Path, PathBuf},
    process::{Command, ExitStatus, Stdio},
    thread,
    time::{Duration, Instant},
};

use super::process_sessions::{self, ProcessControl, ProcessSessions};
use std::sync::Arc;

use crate::permissions::ToolRisk;

use super::{Tool, ToolDefinition, ToolResult};

const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);
const DEFAULT_OUTPUT_LIMIT: usize = 64 * 1024;
const WAIT_INTERVAL: Duration = Duration::from_millis(10);
const CAPTURE_CLEANUP_TIMEOUT: Duration = Duration::from_secs(1);
const MAX_READS_PER_POLL: usize = 16;

pub struct ExecCommandTool {
    workspace: PathBuf,
    timeout: Duration,
    output_limit: usize,
    sessions: Arc<ProcessSessions>,
    control: Option<Arc<ProcessControl>>,
}

impl ExecCommandTool {
    pub fn new(workspace: impl AsRef<Path>) -> Self {
        Self::with_limits(workspace, DEFAULT_TIMEOUT, DEFAULT_OUTPUT_LIMIT)
    }

    pub fn with_limits(
        workspace: impl AsRef<Path>,
        timeout: Duration,
        output_limit: usize,
    ) -> Self {
        Self {
            workspace: workspace.as_ref().to_path_buf(),
            timeout,
            output_limit,
            sessions: Arc::new(ProcessSessions::new(workspace.as_ref(), output_limit)),
            control: None,
        }
    }
    pub(super) fn with_scope(mut self, scope: process_sessions::ProcessScope) -> Self {
        self.sessions = Arc::new(ProcessSessions::for_scope(&self.workspace, scope));
        self
    }
    pub fn session_tools(&self) -> Vec<Box<dyn Tool>> {
        process_sessions::session_tools(self.sessions.clone())
    }
    pub(super) fn with_control(mut self, control: Arc<ProcessControl>) -> Self {
        self.control = Some(control);
        self
    }
}

impl Tool for ExecCommandTool {
    fn name(&self) -> &str {
        "exec_command"
    }

    fn definition(&self) -> ToolDefinition {
        process_sessions::exec_definition()
    }
    fn reset_request_state(&self) {
        self.sessions.clear();
    }
    fn risk(&self, args: &serde_json::Value) -> Result<ToolRisk, String> {
        process_sessions::parse_exec(args)?;
        Ok(ToolRisk::Command)
    }

    fn summarize(&self, args: &serde_json::Value) -> String {
        super::summarize_arguments(
            &serde_json::json!({"command":args["command"], "lifetime":args.get("lifetime").cloned().unwrap_or_else(||serde_json::json!("job")), "workspace":self.workspace, "timeout_ms":args.get("timeout_ms")}),
        )
    }
    fn execute(&self, args: &serde_json::Value) -> Result<ToolResult, String> {
        let args = process_sessions::parse_exec(args)?;
        if (args.yield_time_ms.is_some() || self.sessions.is_scoped()) && self.control.is_none() {
            return self.sessions.start(args);
        }
        if self
            .control
            .as_ref()
            .is_some_and(|control| control.stopped())
        {
            return Ok(ToolResult::text(
                serde_json::json!({"exit_code":null,"timed_out":false,"stopped":true}).to_string(),
            ));
        }
        let timeout = args
            .timeout_ms
            .map(Duration::from_millis)
            .unwrap_or(self.timeout);
        // Validate the caller-provided duration before starting a process. Dropping Child does not
        // terminate it, so no fallible timeout setup may happen after spawn.
        let deadline = Instant::now()
            .checked_add(timeout)
            .ok_or_else(|| "exec_command timeout is too large".to_string())?;

        let mut command = shell_command(&args.command);
        command
            .current_dir(&self.workspace)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        configure_process_group(&mut command);

        let mut child = command
            .spawn()
            .map_err(|error| format!("failed to start exec_command: {error}"))?;
        let process_group = child.id() as i32;
        let capture_result = prepare_capture(&mut child, self.output_limit, self.control.as_ref());
        let (stdout, stderr) = match capture_result {
            Ok(capture) => capture,
            Err(error) => {
                let _ = terminate_child(&mut child, process_group);
                let _ = child.wait();
                return Err(error);
            }
        };

        if let Some(control) = &self.control {
            control.mark_running();
        }
        let execution = match run_and_capture(
            &mut child,
            process_group,
            stdout,
            stderr,
            deadline,
            self.control.as_deref(),
        ) {
            Ok(execution) => execution,
            Err(error) => {
                let _ = terminate_child(&mut child, process_group);
                let _ = child.wait();
                return Err(error);
            }
        };
        let message = serde_json::to_string_pretty(&serde_json::json!({
            "command": args.command,
            "exit_code": if execution.timed_out {
                None
            } else {
                execution.status.and_then(|status| status.code())
            },
            "timed_out": execution.timed_out,
            "stopped": execution.stopped,
            "stdout": String::from_utf8_lossy(&execution.stdout.bytes),
            "stderr": String::from_utf8_lossy(&execution.stderr.bytes),
            "stdout_truncated": execution.stdout.truncated,
            "stderr_truncated": execution.stderr.truncated,
        }))
        .map_err(|error| format!("failed to serialize exec_command result: {error}"))?;

        Ok(ToolResult::text(message))
    }
}

struct CapturedStream<R> {
    reader: R,
    bytes: Vec<u8>,
    limit: usize,
    truncated: bool,
    eof: bool,
    observer: Option<(Arc<ProcessControl>, bool)>,
}

impl<R: Read> CapturedStream<R> {
    fn new(reader: R, limit: usize) -> Self {
        Self {
            reader,
            bytes: Vec::with_capacity(limit.min(8192)),
            limit,
            truncated: false,
            eof: false,
            observer: None,
        }
    }

    fn poll(&mut self) -> Result<(), String> {
        let mut buffer = [0_u8; 8192];
        for _ in 0..MAX_READS_PER_POLL {
            if self.eof {
                break;
            }
            match self.reader.read(&mut buffer) {
                Ok(0) => self.eof = true,
                Ok(read) => {
                    if let Some((control, stderr)) = &self.observer {
                        control.capture(*stderr, &buffer[..read]);
                    }
                    let remaining = self.limit.saturating_sub(self.bytes.len());
                    let to_store = read.min(remaining);
                    self.bytes.extend_from_slice(&buffer[..to_store]);
                    self.truncated |= to_store < read;
                }
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => break,
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                Err(error) => {
                    return Err(format!("failed to read exec_command output: {error}"));
                }
            }
        }
        Ok(())
    }
}

struct CommandExecution<O, E> {
    status: Option<ExitStatus>,
    timed_out: bool,
    stopped: bool,
    stdout: CapturedStream<O>,
    stderr: CapturedStream<E>,
}

fn prepare_capture(
    child: &mut std::process::Child,
    limit: usize,
    control: Option<&Arc<ProcessControl>>,
) -> Result<
    (
        CapturedStream<std::process::ChildStdout>,
        CapturedStream<std::process::ChildStderr>,
    ),
    String,
> {
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| "failed to capture exec_command stdout".to_string())?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| "failed to capture exec_command stderr".to_string())?;
    set_nonblocking(stdout.as_raw_fd())?;
    set_nonblocking(stderr.as_raw_fd())?;
    let mut stdout = CapturedStream::new(stdout, limit);
    let mut stderr = CapturedStream::new(stderr, limit);
    stdout.observer = control.map(|control| (control.clone(), false));
    stderr.observer = control.map(|control| (control.clone(), true));
    Ok((stdout, stderr))
}

fn run_and_capture<O: Read, E: Read>(
    child: &mut std::process::Child,
    process_group: i32,
    mut stdout: CapturedStream<O>,
    mut stderr: CapturedStream<E>,
    deadline: Instant,
    control: Option<&ProcessControl>,
) -> Result<CommandExecution<O, E>, String> {
    loop {
        stdout.poll()?;
        stderr.poll()?;

        // Do not reap the process-group leader until both pipes close. Keeping its PID reserved
        // prevents a later timeout signal from ever targeting a reused process-group ID.
        if stdout.eof && stderr.eof {
            let status = if control.is_some() {
                if exited_without_reaping(child)? {
                    // Reserve the leader PID until all remaining session group members are killed.
                    kill_process_group(process_group)?;
                    Some(
                        child
                            .wait()
                            .map_err(|error| format!("failed to reap exec_command: {error}"))?,
                    )
                } else {
                    None
                }
            } else {
                child
                    .try_wait()
                    .map_err(|error| format!("failed to wait for exec_command: {error}"))?
            };
            if let Some(status) = status {
                return Ok(CommandExecution {
                    status: Some(status),
                    timed_out: false,
                    stopped: false,
                    stdout,
                    stderr,
                });
            }
        }

        let stopped = control.is_some_and(ProcessControl::stopped);
        if stopped || Instant::now() >= deadline {
            terminate_child(child, process_group)?;
            child
                .wait()
                .map_err(|error| format!("failed to reap timed-out exec_command: {error}"))?;

            let cleanup_deadline = Instant::now()
                .checked_add(CAPTURE_CLEANUP_TIMEOUT)
                .unwrap_or_else(Instant::now);
            while !(stdout.eof && stderr.eof) && Instant::now() < cleanup_deadline {
                stdout.poll()?;
                stderr.poll()?;
                if !(stdout.eof && stderr.eof) {
                    thread::sleep(
                        WAIT_INTERVAL
                            .min(cleanup_deadline.saturating_duration_since(Instant::now())),
                    );
                }
            }
            // A descendant may deliberately escape the process group while retaining a pipe.
            // Nonblocking capture lets us return retained output without leaking reader threads.
            stdout.poll()?;
            stderr.poll()?;
            return Ok(CommandExecution {
                status: None,
                timed_out: !stopped,
                stopped,
                stdout,
                stderr,
            });
        }

        thread::sleep(WAIT_INTERVAL.min(deadline.saturating_duration_since(Instant::now())));
    }
}

fn exited_without_reaping(child: &std::process::Child) -> Result<bool, String> {
    // SAFETY: waitid writes to initialized siginfo for our owned child. WNOWAIT leaves
    // the PID reserved, preventing a group signal from targeting a reused PID.
    let mut info: libc::siginfo_t = unsafe { std::mem::zeroed() };
    let result = unsafe {
        libc::waitid(
            libc::P_PID,
            child.id() as libc::id_t,
            &mut info,
            libc::WEXITED | libc::WNOHANG | libc::WNOWAIT,
        )
    };
    if result == -1 {
        let error = io::Error::last_os_error();
        if error.kind() == io::ErrorKind::Interrupted {
            return Ok(false);
        }
        return Err(format!(
            "failed to inspect exec_command completion: {error}"
        ));
    }
    // SAFETY: waitid returned valid siginfo; si_pid is zero when no exit is ready.
    Ok(unsafe { info.si_pid() } != 0)
}

fn set_nonblocking(fd: std::os::fd::RawFd) -> Result<(), String> {
    // SAFETY: fd belongs to an open ChildStdout/ChildStderr for the duration of these calls.
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
    if flags == -1 {
        return Err(format!(
            "failed to inspect exec_command output pipe: {}",
            io::Error::last_os_error()
        ));
    }
    // SAFETY: F_SETFL only updates the status flags for the same valid descriptor.
    if unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } == -1 {
        return Err(format!(
            "failed to configure exec_command output pipe: {}",
            io::Error::last_os_error()
        ));
    }
    Ok(())
}

fn shell_command(command: &str) -> Command {
    let mut shell = Command::new("/bin/sh");
    shell.arg("-c").arg(command);
    shell
}

fn configure_process_group(command: &mut Command) {
    use std::os::unix::process::CommandExt;
    command.process_group(0);
}

fn kill_process_group(process_group: i32) -> Result<(), String> {
    // SAFETY: The child was started in its own process group, and kill only receives its PID.
    let result = unsafe { libc::kill(-process_group, libc::SIGKILL) };
    if result == 0 {
        Ok(())
    } else {
        let error = io::Error::last_os_error();
        if error.raw_os_error() == Some(libc::ESRCH) {
            Ok(())
        } else {
            Err(format!("failed to kill timed-out exec_command: {error}"))
        }
    }
}

fn terminate_child(child: &mut std::process::Child, process_group: i32) -> Result<(), String> {
    let group_result = kill_process_group(process_group);
    let child_result = child
        .kill()
        .map_err(|error| format!("failed to kill timed-out exec_command process: {error}"));

    match (group_result, child_result) {
        (Ok(()), _) | (_, Ok(())) => Ok(()),
        (Err(group_error), Err(child_error)) => Err(format!("{group_error}; {child_error}")),
    }
}

#[cfg(test)]
mod tests {
    use super::CapturedStream;
    use std::io::{self, Read};

    struct AlwaysReadyOutput {
        reads: usize,
    }

    impl Read for AlwaysReadyOutput {
        fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
            self.reads += 1;
            // Fail rather than hanging if capture ever drains a continuous stream without yielding.
            assert!(self.reads < 1_000, "capture must yield to deadline checks");
            buffer.fill(b'x');
            Ok(buffer.len())
        }
    }

    #[test]
    fn capture_poll_yields_and_truncates_continuously_ready_output() {
        let mut stream = CapturedStream::new(AlwaysReadyOutput { reads: 0 }, 1024);

        stream.poll().unwrap();

        assert!(stream.reader.reads > 0);
        assert_eq!(stream.bytes, vec![b'x'; 1024]);
        assert!(stream.truncated);
        assert!(!stream.eof);
    }
}
