use std::{
    io::{BufRead, BufReader, Read, Write},
    path::Path,
    process::{Child, ChildStdin, ChildStdout, Command, Stdio},
    sync::mpsc::{self, Receiver, Sender},
    time::Duration,
};

use serde_json::{json, Value};

use super::config::ServerConfig;

const MAX_FRAME_BYTES: usize = 1024 * 1024;
const PROTOCOL_VERSION: &str = "2025-11-25";

struct Job {
    message: Value,
    reply: Sender<Result<Value, String>>,
}

/// One synchronous session; a pipe worker keeps writes and reads under the same deadline.
pub(super) struct Client {
    child: Child,
    #[cfg(windows)]
    job: Option<super::windows::Job>,
    jobs: Sender<Job>,
    timeout: Duration,
    next_id: u64,
}

impl Client {
    pub fn connect(config: &ServerConfig, workspace: &Path) -> Result<Self, String> {
        let mut command = Command::new(&config.command);
        command
            .args(&config.args)
            .current_dir(workspace)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            // Servers can emit arbitrary sensitive logs. Never forward stderr to diagnostics.
            .stderr(Stdio::null());
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            command.process_group(0);
        }
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            use windows_sys::Win32::System::Threading::{CREATE_NO_WINDOW, CREATE_SUSPENDED};
            command.creation_flags(CREATE_NO_WINDOW | CREATE_SUSPENDED);
        }
        let mut child = command.spawn().map_err(|_| "could not launch MCP server")?;
        #[cfg(windows)]
        let job = super::windows::Job::attach_and_resume(&mut child)?;
        let stdin = child.stdin.take().ok_or("MCP stdin unavailable")?;
        let stdout = child.stdout.take().ok_or("MCP stdout unavailable")?;
        let (jobs, receiver) = mpsc::channel();
        let mut client = Self {
            child,
            #[cfg(windows)]
            job: Some(job),
            jobs,
            timeout: Duration::from_millis(config.timeout_ms),
            next_id: 1,
        };
        std::thread::Builder::new()
            .name("atlas-mcp-stdio".into())
            .spawn(move || worker(stdin, stdout, receiver))
            .map_err(|_| "could not start MCP transport worker")?;
        let initialized = client.request(
            "initialize",
            json!({
                "protocolVersion": PROTOCOL_VERSION,
                "capabilities": {},
                "clientInfo": {"name":"atlas", "version":env!("CARGO_PKG_VERSION")}
            }),
        )?;
        if !matches!(
            initialized["protocolVersion"].as_str(),
            Some("2025-11-25" | "2025-06-18" | "2025-03-26")
        ) {
            return Err("MCP server selected an unsupported protocol version".into());
        }
        if !initialized["capabilities"]["tools"].is_object()
            || !initialized["serverInfo"]["name"].is_string()
            || !initialized["serverInfo"]["version"].is_string()
        {
            return Err(
                "MCP server did not advertise a valid tools capability and identity".into(),
            );
        }
        client.exchange(json!({"jsonrpc":"2.0", "method":"notifications/initialized"}))?;
        Ok(client)
    }

    pub fn request(&mut self, method: &str, params: Value) -> Result<Value, String> {
        let id = self.next_id;
        self.next_id += 1;
        self.exchange(json!({"jsonrpc":"2.0", "id":id, "method":method, "params":params}))
    }

    fn exchange(&mut self, message: Value) -> Result<Value, String> {
        let (reply, response) = mpsc::channel();
        self.jobs
            .send(Job { message, reply })
            .map_err(|_| "MCP connection closed")?;
        response
            .recv_timeout(self.timeout)
            .map_err(|error| match error {
                mpsc::RecvTimeoutError::Timeout => {
                    "MCP request timed out; connection closed; action outcome may be unknown"
                }
                mpsc::RecvTimeoutError::Disconnected => "MCP connection closed",
            })?
    }
}

impl Drop for Client {
    fn drop(&mut self) {
        #[cfg(windows)]
        drop(self.job.take());
        // Also close pipes held by descendants on Unix, including a stalled server's children.
        #[cfg(unix)]
        unsafe {
            libc::kill(-(self.child.id() as i32), libc::SIGKILL);
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn worker(mut stdin: ChildStdin, stdout: ChildStdout, jobs: Receiver<Job>) {
    let mut stdout = BufReader::new(stdout);
    for job in jobs {
        let result =
            write_message(&mut stdin, &job.message).and_then(|()| match job.message.get("id") {
                Some(id) => read_response(&mut stdin, &mut stdout, id),
                None => Ok(Value::Null),
            });
        let failed = result.is_err();
        let _ = job.reply.send(result);
        if failed {
            break;
        }
    }
}

fn write_message(stdin: &mut ChildStdin, message: &Value) -> Result<(), String> {
    let mut bytes = serde_json::to_vec(message).map_err(|_| "could not encode MCP request")?;
    bytes.push(b'\n');
    if bytes.len() > MAX_FRAME_BYTES {
        return Err("MCP request exceeds the 1 MiB message limit".into());
    }
    stdin
        .write_all(&bytes)
        .and_then(|()| stdin.flush())
        .map_err(|_| "could not write MCP request".into())
}

fn read_response(
    stdin: &mut ChildStdin,
    stdout: &mut BufReader<ChildStdout>,
    expected_id: &Value,
) -> Result<Value, String> {
    // The caller's deadline also bounds notification floods and blocked pipe writes.
    loop {
        let mut line = Vec::new();
        (&mut *stdout)
            .take((MAX_FRAME_BYTES + 1) as u64)
            .read_until(b'\n', &mut line)
            .map_err(|_| "could not read MCP response")?;
        if line.len() > MAX_FRAME_BYTES {
            return Err("MCP response exceeds the 1 MiB message limit".into());
        }
        if line.last() != Some(&b'\n') {
            return Err("MCP server closed its output".into());
        }
        let message: Value =
            serde_json::from_slice(&line).map_err(|_| "invalid MCP JSON response")?;
        if message["jsonrpc"] != "2.0" || !message.is_object() {
            return Err("invalid MCP response envelope".into());
        }
        if let Some(method) = message.get("method") {
            let method = method.as_str().ok_or("invalid MCP method")?;
            if message.get("result").is_some() || message.get("error").is_some() {
                return Err("invalid MCP request envelope".into());
            }
            if let Some(id) = message.get("id") {
                // No sampling, elicitation or filesystem authority is delegated to a server.
                let response = if method == "ping" {
                    json!({"jsonrpc":"2.0", "id":id, "result":{}})
                } else {
                    json!({"jsonrpc":"2.0", "id":id, "error":{"code":-32601,"message":"Unsupported client method"}})
                };
                write_message(stdin, &response)?;
            }
            continue;
        }
        if message.get("id") != Some(expected_id)
            || message.get("result").is_some() == message.get("error").is_some()
        {
            return Err("invalid MCP response identity or result".into());
        }
        if message.get("error").is_some() {
            return Err("MCP server returned a protocol error".into());
        }
        return message
            .get("result")
            .filter(|result| result.is_object())
            .cloned()
            .ok_or_else(|| "invalid MCP result".into());
    }
}
