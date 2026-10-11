use std::{
    io::{self, Read},
    os::fd::AsRawFd,
    path::{Path, PathBuf},
    process::{Command, ExitStatus, Stdio},
    thread,
    time::{Duration, Instant},
};

use serde::Deserialize;

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
        }
    }
}

impl Tool for ExecCommandTool {
    fn name(&self) -> &str {
        "exec_command"
    }

    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: self.name().to_string(),
            description: "Execute an unrestricted shell command from the Atlas workspace. Supports shell operators, pipes, redirects, and access outside the workspace."
                .to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "required": ["command"],
                "properties": {
                    "command": {
                        "type": "string",
                        "description": "The complete shell command to execute."
                    }
                }
            }),
        }
    }

    fn risk(&self, args: &serde_json::Value) -> Result<ToolRisk, String> {
        let args: ExecCommandArgs = serde_json::from_value(args.clone())
            .map_err(|error| format!("invalid exec_command args: {error}"))?;
        if args.command.trim().is_empty() {
            return Err("exec_command command is required".to_string());
        }
        Ok(ToolRisk::Command)
    }

    fn execute(&self, args: &serde_json::Value) -> Result<ToolResult, String> {
        let args: ExecCommandArgs = serde_json::from_value(args.clone())
            .map_err(|error| format!("invalid exec_command args: {error}"))?;
        if args.command.trim().is_empty() {
            return Err("exec_command command is required".to_string());
        }

        // Validate the caller-provided duration before starting a process. Dropping Child does not
        // terminate it, so no fallible timeout setup may happen after spawn.
        let deadline = Instant::now()
            .checked_add(self.timeout)
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
        let capture_result = prepare_capture(&mut child, self.output_limit);
        let (stdout, stderr) = match capture_result {
            Ok(capture) => capture,
            Err(error) => {
                let _ = terminate_child(&mut child, process_group);
                let _ = child.wait();
                return Err(error);
            }
        };

        let execution = match run_and_capture(&mut child, process_group, stdout, stderr, deadline) {
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
            "stdout": String::from_utf8_lossy(&execution.stdout.bytes),
            "stderr": String::from_utf8_lossy(&execution.stderr.bytes),
            "stdout_truncated": execution.stdout.truncated,
            "stderr_truncated": execution.stderr.truncated,
        }))
        .map_err(|error| format!("failed to serialize exec_command result: {error}"))?;

        Ok(ToolResult::text(message))
    }
}

#[derive(Deserialize)]
struct ExecCommandArgs {
    command: String,
}

struct CapturedStream<R> {
    reader: R,
    bytes: Vec<u8>,
    limit: usize,
    truncated: bool,
    eof: bool,
}

impl<R: Read> CapturedStream<R> {
    fn new(reader: R, limit: usize) -> Self {
        Self {
            reader,
            bytes: Vec::with_capacity(limit.min(8192)),
            limit,
            truncated: false,
            eof: false,
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
    stdout: CapturedStream<O>,
    stderr: CapturedStream<E>,
}

fn prepare_capture(
    child: &mut std::process::Child,
    limit: usize,
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
    Ok((
        CapturedStream::new(stdout, limit),
        CapturedStream::new(stderr, limit),
    ))
}

fn run_and_capture<O: Read, E: Read>(
    child: &mut std::process::Child,
    process_group: i32,
    mut stdout: CapturedStream<O>,
    mut stderr: CapturedStream<E>,
    deadline: Instant,
) -> Result<CommandExecution<O, E>, String> {
    loop {
        stdout.poll()?;
        stderr.poll()?;

        // Do not reap the process-group leader until both pipes close. Keeping its PID reserved
        // prevents a later timeout signal from ever targeting a reused process-group ID.
        if stdout.eof && stderr.eof {
            if let Some(status) = child
                .try_wait()
                .map_err(|error| format!("failed to wait for exec_command: {error}"))?
            {
                return Ok(CommandExecution {
                    status: Some(status),
                    timed_out: false,
                    stdout,
                    stderr,
                });
            }
        }

        if Instant::now() >= deadline {
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
                timed_out: true,
                stdout,
                stderr,
            });
        }

        thread::sleep(WAIT_INTERVAL.min(deadline.saturating_duration_since(Instant::now())));
    }
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
