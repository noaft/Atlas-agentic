//! Local backend diagnostics. Never pass prompts, response bodies, or tool payloads here.
use std::{
    cell::RefCell,
    fs::{self, OpenOptions},
    io::{self, Write},
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        Mutex, OnceLock,
    },
    time::{Duration, Instant},
};

use fs2::FileExt;
use serde_json::{json, Value};

const MAX_FILE_BYTES: u64 = 5 * 1024 * 1024;
const BACKUP_COUNT: usize = 3;
static LOGGER: OnceLock<BackendLog> = OnceLock::new();
static WRITE_WARNING: AtomicBool = AtomicBool::new(false);
thread_local! { static REQUEST_ID: RefCell<Option<String>> = const { RefCell::new(None) }; }

struct BackendLog {
    directory: PathBuf,
    component: &'static str,
    secrets: Mutex<Vec<String>>,
    max_bytes: u64,
    stderr: bool,
}

impl BackendLog {
    fn new(directory: PathBuf, component: &'static str, stderr: bool) -> io::Result<Self> {
        let mut builder = fs::DirBuilder::new();
        builder.recursive(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            builder.mode(0o700);
        }
        builder.create(&directory)?;
        // Check writability at startup, even before the first event.
        private_options()
            .append(true)
            .create(true)
            .open(directory.join("atlas.jsonl"))?;
        Ok(Self {
            directory,
            component,
            secrets: Mutex::new(Vec::new()),
            max_bytes: MAX_FILE_BYTES,
            stderr,
        })
    }

    fn protect(&self, secret: &str) {
        if secret.is_empty() {
            return;
        }
        if let Ok(mut secrets) = self.secrets.lock() {
            if !secrets.iter().any(|value| value == secret) {
                secrets.push(secret.to_owned());
            }
        }
    }

    fn write(
        &self,
        level: &str,
        event: &str,
        request_id: Option<&str>,
        mut fields: Value,
    ) -> io::Result<()> {
        let secrets = self
            .secrets
            .lock()
            .map_err(|_| io::Error::other("diagnostic redaction lock poisoned"))?;
        sanitize_fields(&mut fields, &secrets);
        drop(secrets);
        let record = json!({
            "timestamp": chrono::Utc::now().to_rfc3339(),
            "level": level, "event": event, "component": self.component,
            "pid": std::process::id(), "requestId": request_id, "fields": fields,
        });
        let mut line = serde_json::to_vec(&record)?;
        line.push(b'\n');
        let lock = private_options()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(self.directory.join("atlas.lock"))?;
        acquire_log_lock(&lock)?;
        let path = self.directory.join("atlas.jsonl");
        let length = match fs::metadata(&path) {
            Ok(metadata) => metadata.len(),
            Err(error) if error.kind() == io::ErrorKind::NotFound => 0,
            Err(error) => return Err(error),
        };
        if length > 0 && length.saturating_add(line.len() as u64) > self.max_bytes {
            self.rotate()?;
        }
        let mut file = private_options().append(true).create(true).open(path)?;
        file.write_all(&line)?;
        // Files are opened per write so another Atlas process can rotate safely.
        drop(file);
        drop(lock);
        if self.stderr {
            let _ = io::stderr().lock().write_all(&line);
        }
        Ok(())
    }

    fn rotate(&self) -> io::Result<()> {
        let oldest = self.directory.join(format!("atlas.{BACKUP_COUNT}.jsonl"));
        if oldest.exists() {
            fs::remove_file(oldest)?;
        }
        for index in (1..BACKUP_COUNT).rev() {
            let source = self.directory.join(format!("atlas.{index}.jsonl"));
            if source.exists() {
                fs::rename(
                    source,
                    self.directory.join(format!("atlas.{}.jsonl", index + 1)),
                )?;
            }
        }
        fs::rename(
            self.directory.join("atlas.jsonl"),
            self.directory.join("atlas.1.jsonl"),
        )
    }
}

fn acquire_log_lock(file: &fs::File) -> io::Result<()> {
    let started = Instant::now();
    loop {
        match file.try_lock_exclusive() {
            Ok(()) => return Ok(()),
            Err(error)
                if error.kind() == io::ErrorKind::WouldBlock
                    || error.raw_os_error() == fs2::lock_contended_error().raw_os_error() =>
            {
                if started.elapsed() >= Duration::from_millis(50) {
                    return Err(error);
                }
                std::thread::sleep(Duration::from_millis(1));
            }
            Err(error) => return Err(error),
        }
    }
}

fn private_options() -> OpenOptions {
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        let mut options = OpenOptions::new();
        options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
        options
    }
    #[cfg(not(unix))]
    {
        OpenOptions::new()
    }
}

fn sanitize_fields(value: &mut Value, secrets: &[String]) {
    match value {
        Value::String(text) => {
            for secret in secrets {
                *text = text.replace(secret, "[redacted]");
            }
            // Network errors can contain endpoint credentials or query parameters.
            *text = text
                .split_whitespace()
                .map(|word| {
                    if word.contains("://") {
                        "[endpoint]"
                    } else {
                        word
                    }
                })
                .collect::<Vec<_>>()
                .join(" ");
            if text.chars().count() > 2048 {
                *text = text.chars().take(2048).collect::<String>() + "…";
            }
        }
        Value::Object(fields) => {
            for (key, value) in fields {
                let key = key.to_ascii_lowercase();
                if ["key", "token", "secret", "password", "authorization"]
                    .iter()
                    .any(|part| key.contains(part))
                {
                    *value = Value::String("[redacted]".into());
                } else {
                    if key == "error" {
                        if let Value::String(error) = value {
                            for marker in [
                                "invalid agent tool protocol:",
                                "unknown variant",
                                "unknown field",
                                "invalid type:",
                                "invalid value:",
                            ] {
                                if let Some(offset) = error.find(marker) {
                                    *error = format!(
                                        "{}{marker} [response validation details omitted]",
                                        &error[..offset]
                                    );
                                    break;
                                }
                            }
                        }
                    }
                    sanitize_fields(value, secrets);
                }
            }
        }
        Value::Array(values) => {
            for value in values {
                sanitize_fields(value, secrets);
            }
        }
        _ => {}
    }
}

fn log_directory() -> Result<PathBuf, String> {
    if let Some(path) = std::env::var_os("ATLAS_LOG_DIR") {
        return Ok(path.into());
    }
    let root = match std::env::var_os("ATLAS_DATA_DIR") {
        Some(root) => PathBuf::from(root),
        None => directories::BaseDirs::new()
            .ok_or("could not resolve the local data directory")?
            .data_local_dir()
            .join("atlas"),
    };
    Ok(root.join("logs"))
}

/// Initialize once at process startup. Logging errors never prevent Atlas from running.
pub fn init(component: &'static str) {
    let result = log_directory().and_then(|directory| {
        BackendLog::new(
            directory,
            component,
            std::env::var_os("ATLAS_LOG_STDERR").is_some_and(|v| v == "1"),
        )
        .map_err(|error| error.to_string())
    });
    match result {
        Ok(logger) => {
            for (name, value) in std::env::vars_os() {
                let name = name.to_string_lossy().to_ascii_uppercase();
                if ["API_KEY", "TOKEN", "SECRET", "PASSWORD"]
                    .iter()
                    .any(|part| name.contains(part))
                {
                    logger.protect(&value.to_string_lossy());
                }
            }
            if LOGGER.set(logger).is_ok() {
                event(
                    "INFO",
                    "application.started",
                    json!({"version": env!("CARGO_PKG_VERSION")}),
                );
            }
        }
        Err(error) => eprintln!("Atlas backend logging unavailable: {error}"),
    }
}

/// Register saved credentials before constructing or calling a provider.
pub fn protect_secret(secret: &str) {
    if let Some(logger) = LOGGER.get() {
        logger.protect(secret);
    }
}

pub fn event(level: &str, name: &str, fields: Value) {
    if let Some(logger) = LOGGER.get() {
        let result =
            REQUEST_ID.with(|id| logger.write(level, name, id.borrow().as_deref(), fields));
        if result.is_err() && !WRITE_WARNING.swap(true, Ordering::Relaxed) {
            eprintln!(
                "Atlas backend log write failed; check directory access, disk space, or contention from another Atlas process"
            );
        }
    }
}

/// Correlate synchronous backend work, including nested model calls, on its worker thread.
pub fn run<T>(
    operation: &'static str,
    fields: Value,
    work: impl FnOnce() -> Result<T, String>,
) -> Result<T, String> {
    let previous = REQUEST_ID.with(|id| {
        let current = id
            .borrow()
            .clone()
            .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
        id.replace(Some(current))
    });
    let mut trace = Operation {
        operation,
        previous,
        started: Instant::now(),
        finished: false,
    };
    event(
        "INFO",
        "operation.started",
        json!({"operation": operation, "context": fields}),
    );
    let result = work();
    let duration = u64::try_from(trace.started.elapsed().as_millis()).unwrap_or(u64::MAX);
    match &result {
        Ok(_) => event(
            "INFO",
            "operation.succeeded",
            json!({"operation": operation, "durationMs": duration}),
        ),
        Err(error) => event(
            "ERROR",
            "operation.failed",
            json!({"operation": operation, "durationMs": duration, "error": error}),
        ),
    }
    trace.finished = true;
    result
}

struct Operation {
    operation: &'static str,
    previous: Option<String>,
    started: Instant,
    finished: bool,
}
impl Drop for Operation {
    fn drop(&mut self) {
        if !self.finished {
            event(
                "ERROR",
                "operation.aborted",
                json!({"operation": self.operation, "reason": "worker unwound before completion"}),
            );
        }
        REQUEST_ID.with(|id| id.replace(self.previous.take()));
    }
}

/// Safe model configuration metadata; endpoint URLs and credentials never become fields.
pub(crate) fn provider_settings(settings: &crate::settings::EffectiveSettings) {
    if let Some(secret) = &settings.api_key {
        protect_secret(secret);
    }
    event(
        "INFO",
        "provider.configured",
        json!({"provider": settings.provider.as_str(), "model": settings.model}),
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[test]
    fn records_are_live_private_bounded_and_redacted() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("logs");
        let log = BackendLog::new(path.clone(), "test", false).unwrap();
        log.protect("saved-test-key");
        log.write(
            "ERROR",
            "operation.failed",
            Some("request-1"),
            json!({
                "error": "HTTP 401 saved-test-key https://user:password@local.invalid/?key=hidden",
                "apiKey": "another-secret",
                "detail": "x".repeat(5000),
            }),
        )
        .unwrap();
        let text = fs::read_to_string(path.join("atlas.jsonl")).unwrap();
        for secret in ["saved-test-key", "another-secret", "password", "hidden"] {
            assert!(!text.contains(secret));
        }
        let record: Value = serde_json::from_str(text.trim()).unwrap();
        assert_eq!(record["requestId"], "request-1");
        assert!(record["fields"]["error"]
            .as_str()
            .unwrap()
            .contains("HTTP 401"));
        assert!(record["fields"]["detail"].as_str().unwrap().chars().count() <= 2049);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(&path).unwrap().permissions().mode() & 0o777,
                0o700
            );
            for name in ["atlas.jsonl", "atlas.lock"] {
                assert_eq!(
                    fs::metadata(path.join(name)).unwrap().permissions().mode() & 0o777,
                    0o600
                );
            }
        }
    }

    #[test]
    fn response_derived_parse_errors_do_not_leak_model_text() {
        let root = tempfile::tempdir().unwrap();
        let log = BackendLog::new(root.path().to_owned(), "test", false).unwrap();
        let error = crate::agent_output::AgentOutput::parse_tool_protocol(
            r#"{"type":"private model response fragment"}"#,
        )
        .unwrap_err();
        log.write("ERROR", "operation.failed", None, json!({"error": error}))
            .unwrap();
        let text = fs::read_to_string(root.path().join("atlas.jsonl")).unwrap();
        assert!(!text.contains("private model response fragment"));
        assert!(text.contains("invalid agent tool protocol"));
    }

    #[test]
    fn a_contended_log_lock_does_not_stall_application_work() {
        let root = tempfile::tempdir().unwrap();
        let log = BackendLog::new(root.path().to_owned(), "test", false).unwrap();
        let lock = private_options()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(root.path().join("atlas.lock"))
            .unwrap();
        lock.lock_exclusive().unwrap();
        let (sender, receiver) = std::sync::mpsc::channel();
        let worker = std::thread::spawn(move || {
            sender
                .send(log.write("INFO", "contended", None, json!({})))
                .unwrap();
        });
        let result = receiver.recv_timeout(std::time::Duration::from_millis(250));
        drop(lock);
        worker.join().unwrap();
        assert!(
            result.is_ok(),
            "log lock acquisition must have a bounded wait"
        );
        assert!(result.unwrap().is_err());
    }

    #[test]
    fn rotation_keeps_only_three_backups_and_resumes_after_external_removal() {
        let root = tempfile::tempdir().unwrap();
        let mut log = BackendLog::new(root.path().to_owned(), "test", false).unwrap();
        log.max_bytes = 500;
        for number in 0..30 {
            log.write("INFO", "test", None, json!({"number": number}))
                .unwrap();
        }
        for name in [
            "atlas.jsonl",
            "atlas.1.jsonl",
            "atlas.2.jsonl",
            "atlas.3.jsonl",
        ] {
            let file = root.path().join(name);
            assert!(fs::metadata(&file).unwrap().len() <= 500);
            for line in fs::read_to_string(file).unwrap().lines() {
                serde_json::from_str::<Value>(line).unwrap();
            }
        }
        assert!(!root.path().join("atlas.4.jsonl").exists());
        fs::remove_file(root.path().join("atlas.jsonl")).unwrap();
        log.write("INFO", "resumed", None, json!({})).unwrap();
        assert!(fs::read_to_string(root.path().join("atlas.jsonl"))
            .unwrap()
            .contains("resumed"));
    }

    #[test]
    fn independent_writers_do_not_interleave_records() {
        let root = tempfile::tempdir().unwrap();
        let logs: Vec<_> = (0..4)
            .map(|_| Arc::new(BackendLog::new(root.path().to_owned(), "test", false).unwrap()))
            .collect();
        let threads: Vec<_> = logs
            .into_iter()
            .map(|log| {
                std::thread::spawn(move || {
                    for number in 0..50 {
                        log.write("INFO", "parallel", None, json!({"number": number}))
                            .unwrap();
                    }
                })
            })
            .collect();
        for thread in threads {
            thread.join().unwrap();
        }
        let text = fs::read_to_string(root.path().join("atlas.jsonl")).unwrap();
        assert_eq!(text.lines().count(), 200);
        for line in text.lines() {
            serde_json::from_str::<Value>(line).unwrap();
        }
    }

    #[test]
    fn request_ids_nest_and_reset_after_failure_or_panic() {
        let result: Result<(), String> = run("outer", json!({}), || {
            let id = REQUEST_ID.with(|id| id.borrow().clone()).unwrap();
            run("inner", json!({}), || {
                assert_eq!(REQUEST_ID.with(|id| id.borrow().clone()), Some(id));
                Ok(())
            })?;
            Err("test failure".into())
        });
        assert!(result.is_err());
        assert!(REQUEST_ID.with(|id| id.borrow().is_none()));
        let _ = std::panic::catch_unwind(|| run::<()>("panic", json!({}), || panic!("test")));
        assert!(REQUEST_ID.with(|id| id.borrow().is_none()));
    }
}
