use std::{
    mem::{size_of, zeroed},
    os::windows::io::AsRawHandle,
    os::windows::process::CommandExt,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    ptr, thread,
    time::{Duration, Instant},
};
use windows_sys::Win32::{
    Foundation::{CloseHandle, HANDLE},
    System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation,
        SetInformationJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
        JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
    },
    System::Threading::CREATE_SUSPENDED,
};

use super::process_sessions::{self, ProcessControl, ProcessSessions};
use std::sync::Arc;

use crate::permissions::ToolRisk;

use super::{output_capture::capture_bounded_observed, Tool, ToolDefinition, ToolResult};

#[link(name = "ntdll")]
extern "system" {
    fn NtResumeProcess(process_handle: HANDLE) -> i32;
}

const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);
const DEFAULT_OUTPUT_LIMIT: usize = 64 * 1024;
const WAIT_INTERVAL: Duration = Duration::from_millis(10);

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
    pub fn with_process_limits(
        self,
        limits: crate::process_limits::ProcessLimits,
    ) -> Result<Self, String> {
        self.sessions.set_limits(limits)?;
        Ok(self)
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
        // CLI synchronous execution keeps its original output/timeout behavior,
        // but reserves capacity alongside yielded sessions until this call exits.
        let _admission = if self.control.is_none() {
            Some(self.sessions.admit_synchronous()?)
        } else {
            None
        };
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
        let deadline = Instant::now()
            .checked_add(timeout)
            .ok_or_else(|| "exec_command timeout is too large".to_string())?;
        let mut child = Command::new("cmd.exe")
            .args(["/D", "/S", "/C"])
            // cmd.exe does not use C-runtime argument escaping. Preserve the approved
            // shell text, including inner quotes, inside its required outer quotes.
            .raw_arg(format!("\"{}\"", args.command))
            .current_dir(&self.workspace)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .creation_flags(CREATE_SUSPENDED)
            .spawn()
            .map_err(|error| format!("failed to start exec_command: {error}"))?;
        let job = Job::new().and_then(|job| {
            job.assign(child.as_raw_handle() as HANDLE)?;
            Ok(job)
        });
        let mut job = match job {
            Ok(job) => Some(job),
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(error);
            }
        };
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| "failed to capture exec_command stdout".to_string())?;
        let stderr = child
            .stderr
            .take()
            .ok_or_else(|| "failed to capture exec_command stderr".to_string())?;
        let resumed = unsafe { NtResumeProcess(child.as_raw_handle() as HANDLE) };
        if resumed < 0 {
            drop(job.take());
            let _ = child.wait();
            return Err(format!(
                "failed to resume exec_command process: NTSTATUS {resumed:#010x}"
            ));
        }
        if let Some(control) = &self.control {
            control.mark_running();
        }
        let limit = self.output_limit;
        let stdout_control = self.control.clone();
        let stderr_control = self.control.clone();
        let stdout_reader = thread::spawn(move || {
            capture_bounded_observed(stdout, limit, |bytes| {
                if let Some(control) = &stdout_control {
                    control.capture(false, bytes);
                }
            })
        });
        let stderr_reader = thread::spawn(move || {
            capture_bounded_observed(stderr, limit, |bytes| {
                if let Some(control) = &stderr_control {
                    control.capture(true, bytes);
                }
            })
        });

        let (status, timed_out, stopped) = loop {
            if let Some(status) = child
                .try_wait()
                .map_err(|error| format!("failed to wait for exec_command: {error}"))?
            {
                break (Some(status), false, false);
            }
            let stopped = self
                .control
                .as_ref()
                .is_some_and(|control| control.stopped());
            if stopped || Instant::now() >= deadline {
                drop(job.take());
                child
                    .wait()
                    .map_err(|error| format!("failed to reap timed-out exec_command: {error}"))?;
                break (None, !stopped, stopped);
            }
            thread::sleep(WAIT_INTERVAL.min(deadline.saturating_duration_since(Instant::now())));
        };

        drop(job.take());

        let stdout = stdout_reader
            .join()
            .map_err(|_| "exec_command stdout reader panicked".to_string())??;
        let stderr = stderr_reader
            .join()
            .map_err(|_| "exec_command stderr reader panicked".to_string())??;
        let message = serde_json::to_string_pretty(&serde_json::json!({
            "command": args.command,
            "exit_code": if timed_out { None } else { status.and_then(|value| value.code()) },
            "timed_out": timed_out,
            "stopped": stopped,
            "stdout": String::from_utf8_lossy(&stdout.0),
            "stderr": String::from_utf8_lossy(&stderr.0),
            "stdout_truncated": stdout.1,
            "stderr_truncated": stderr.1,
        }))
        .map_err(|error| format!("failed to serialize exec_command result: {error}"))?;
        Ok(ToolResult::text(message))
    }
}

struct Job(HANDLE);

impl Job {
    fn new() -> Result<Self, String> {
        let handle = unsafe { CreateJobObjectW(ptr::null(), ptr::null()) };
        if handle.is_null() {
            return Err(format!(
                "failed to create exec_command job object: {}",
                std::io::Error::last_os_error()
            ));
        }
        let job = Self(handle);
        let mut information: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = unsafe { zeroed() };
        information.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        let configured = unsafe {
            SetInformationJobObject(
                job.0,
                JobObjectExtendedLimitInformation,
                &information as *const _ as *const _,
                size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            )
        };
        if configured == 0 {
            return Err(format!(
                "failed to configure exec_command job object: {}",
                std::io::Error::last_os_error()
            ));
        }
        Ok(job)
    }

    fn assign(&self, process: HANDLE) -> Result<(), String> {
        if unsafe { AssignProcessToJobObject(self.0, process) } == 0 {
            return Err(format!(
                "failed to assign exec_command process to job object: {}",
                std::io::Error::last_os_error()
            ));
        }
        Ok(())
    }
}

impl Drop for Job {
    fn drop(&mut self) {
        unsafe {
            CloseHandle(self.0);
        }
    }
}
