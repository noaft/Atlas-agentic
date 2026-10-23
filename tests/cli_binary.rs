use std::{
    io::{BufRead, BufReader, Read, Write},
    net::{TcpListener, TcpStream},
    process::{Command, Output, Stdio},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver},
        Arc,
    },
    thread::{self, JoinHandle},
    time::Duration,
};

use atlas_agentic::{
    conversations::{ConversationStore, MessageRole},
    permissions::PermissionMode,
    settings::{AtlasSettings, SettingsStore},
};
use serde_json::{json, Value};
use tempfile::TempDir;

struct CliFixture {
    workspace: TempDir,
    data: TempDir,
    config: TempDir,
}

impl CliFixture {
    fn new() -> Self {
        Self {
            workspace: tempfile::tempdir().unwrap(),
            data: tempfile::tempdir().unwrap(),
            config: tempfile::tempdir().unwrap(),
        }
    }

    fn store(&self) -> ConversationStore {
        ConversationStore::at(self.data.path().to_path_buf())
    }

    fn command(&self) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_atlas"));
        command
            .current_dir(self.workspace.path())
            .env_clear()
            .env("ATLAS_SKIP_DOTENV", "1")
            .env("ATLAS_DATA_DIR", self.data.path())
            .env("ATLAS_CONFIG_DIR", self.config.path());
        command
    }

    fn install_agent(&self) {
        let agents = self.workspace.path().join("agents");
        std::fs::create_dir(&agents).unwrap();
        std::fs::write(
            agents.join("atlas.toml"),
            include_str!("../agents/atlas.toml"),
        )
        .unwrap();
    }

    fn ask_command(&self, server: &FakeDeepSeek) -> Command {
        let mut command = self.command();
        command
            .env("DEEPSEEK_API_KEY", "fake_key_for_test")
            .env("DEEPSEEK_BASE_URL", &server.url);
        command
    }

    fn settings_store(&self) -> SettingsStore {
        SettingsStore::at(self.config.path().join("settings.json"))
    }
}

fn successful_stdout(output: Output) -> String {
    assert!(
        output.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr
            .lines()
            .all(|line| line.starts_with("Context: ") || line.starts_with("Compacting context: ")),
        "{stderr}"
    );
    String::from_utf8(output.stdout).unwrap()
}

fn output_with_stdin(mut command: Command, input: &str) -> Output {
    let mut child = command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(input.as_bytes())
        .unwrap();
    child.wait_with_output().unwrap()
}

#[test]
fn help_command_does_not_require_deepseek_key_or_agent_file() {
    let fixture = CliFixture::new();
    let stdout = successful_stdout(fixture.command().arg("help").output().unwrap());
    for usage in [
        "Atlas Agentic",
        "--conversation <id>",
        "conversation list",
        "conversation show <id>",
        "conversation rename <id>",
        "conversation delete <id>",
    ] {
        assert!(stdout.contains(usage), "missing {usage}: {stdout}");
    }
}

#[test]
fn startup_and_failures_are_written_to_a_local_backend_log() {
    let fixture = CliFixture::new();
    let output = fixture.command().arg("unknown").output().unwrap();
    assert!(!output.status.success());
    let log = std::fs::read_to_string(fixture.data.path().join("logs/atlas.jsonl")).unwrap();
    let events: Vec<Value> = log
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert!(events
        .iter()
        .any(|e| e["event"] == "application.started" && e["component"] == "cli"));
    let failed = events
        .iter()
        .find(|e| e["event"] == "operation.failed")
        .unwrap();
    assert_eq!(failed["fields"]["operation"], "cli.command");
    assert!(failed["fields"]["error"]
        .as_str()
        .unwrap()
        .contains("unknown command"));
    assert!(failed["requestId"].as_str().is_some());
    assert!(failed["fields"]["durationMs"].as_u64().is_some());
    assert!(failed["timestamp"].as_str().is_some());
}

#[test]
fn log_location_can_be_overridden_and_logging_failure_does_not_block_startup() {
    let fixture = CliFixture::new();
    let logs = TempDir::new().unwrap();
    successful_stdout(
        fixture
            .command()
            .env("ATLAS_LOG_DIR", logs.path())
            .arg("help")
            .output()
            .unwrap(),
    );
    assert!(logs.path().join("atlas.jsonl").exists());
    assert!(!fixture.data.path().join("logs").exists());
    let blocked = logs.path().join("not-a-directory");
    std::fs::write(&blocked, "occupied").unwrap();
    let output = fixture
        .command()
        .env("ATLAS_LOG_DIR", &blocked)
        .arg("help")
        .output()
        .unwrap();
    assert!(output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("backend logging unavailable"));
}

#[test]
fn model_failure_logs_correlated_errors_without_credentials_or_prompt() {
    let fixture = CliFixture::new();
    fixture.install_agent();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let endpoint = format!("http://{}", listener.local_addr().unwrap());
    let server = thread::spawn(move || loop {
        let (mut socket, _) = listener.accept().unwrap();
        socket
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        let request = read_request(&mut socket);
        // Context budgeting may discover model limits before the chat request.
        if request["models"] == true {
            let body = r#"{"data":[]}"#;
            write!(socket, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
            continue;
        }
        socket.write_all(b"HTTP/1.1 503 Service Unavailable\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").unwrap();
        break;
    });
    let output = fixture
        .command()
        .env("DEEPSEEK_API_KEY", "fake_key_for_test")
        .env("DEEPSEEK_BASE_URL", endpoint)
        .args(["ask", "private prompt body"])
        .output()
        .unwrap();
    server.join().unwrap();
    assert!(!output.status.success());
    let log = std::fs::read_to_string(fixture.data.path().join("logs/atlas.jsonl")).unwrap();
    assert!(!log.contains("fake_key_for_test"));
    assert!(!log.contains("private prompt body"));
    let events: Vec<Value> = log
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert!(events
        .iter()
        .any(|e| e["event"] == "provider.configured" && e["fields"]["provider"] == "deepseek"));
    let failed: Vec<_> = events
        .iter()
        .filter(|e| e["event"] == "operation.failed")
        .collect();
    assert!(failed
        .iter()
        .any(|e| e["fields"]["operation"] == "model.chat"));
    assert!(failed
        .iter()
        .any(|e| e["fields"]["operation"] == "cli.command"));
    assert!(failed
        .iter()
        .all(|e| e["requestId"] == failed[0]["requestId"]));
    assert!(
        failed
            .iter()
            .all(|e| e["fields"]["error"].as_str().unwrap().contains("503")),
        "{log}"
    );
}

#[test]
fn invalid_commands_are_reported_before_provider_setup() {
    let fixture = CliFixture::new();
    for (args, expected) in [
        (vec!["unknown"], "unknown command: unknown"),
        (
            vec!["ask", "--conversation", "invalid", "hello"],
            "invalid conversation ID",
        ),
        (vec!["ask"], "ask needs a prompt"),
    ] {
        let output = fixture.command().args(args).output().unwrap();
        assert!(!output.status.success());
        let stderr = String::from_utf8(output.stderr).unwrap();
        assert!(stderr.contains(expected), "{stderr}");
        assert!(!stderr.contains("DEEPSEEK_API_KEY"));
    }
}

#[test]
fn list_displays_empty_store_without_provider_setup() {
    let fixture = CliFixture::new();
    assert_eq!(
        successful_stdout(
            fixture
                .command()
                .args(["conversation", "list"])
                .output()
                .unwrap()
        ),
        "No conversations.\n"
    );
}

#[test]
fn management_commands_render_and_update_local_conversations_without_provider_setup() {
    let fixture = CliFixture::new();
    let store = fixture.store();
    let older = store.create_turn("Older question", "Older answer").unwrap();
    let newer = store.create_turn("Newer question", "Newer answer").unwrap();
    let stdout = successful_stdout(
        fixture
            .command()
            .args(["conversation", "list"])
            .output()
            .unwrap(),
    );
    assert!(
        stdout.find(&newer.id.to_string()).unwrap() < stdout.find(&older.id.to_string()).unwrap()
    );
    for metadata in [&older, &newer] {
        assert!(stdout.contains(&format!(
            "Conversation: {} | Untitled conversation",
            metadata.id
        )));
        assert!(stdout.contains(&format!("Created: {}", metadata.created_at.to_rfc3339())));
        assert!(stdout.contains(&format!("Updated: {}", metadata.updated_at.to_rfc3339())));
    }
    assert_eq!(stdout.matches("Messages: 2").count(), 2);
    let id = newer.id.to_string();
    let stdout = successful_stdout(
        fixture
            .command()
            .args(["conversation", "show", &id])
            .output()
            .unwrap(),
    );
    assert!(stdout.contains(&format!("Conversation: {id} | Untitled conversation")));
    assert!(stdout.contains(&format!(
        "user [{}]\nNewer question",
        newer.created_at.to_rfc3339()
    )));
    assert!(stdout.contains(&format!(
        "assistant [{}]\nNewer answer",
        newer.created_at.to_rfc3339()
    )));
    let stdout = successful_stdout(
        fixture
            .command()
            .args(["conversation", "rename", &id, "Atlas runtime work"])
            .output()
            .unwrap(),
    );
    assert_eq!(
        stdout,
        format!("Renamed conversation: {id} | Atlas runtime work\n")
    );
    assert_eq!(
        store.load(&newer.id).unwrap().metadata.title,
        "Atlas runtime work"
    );
    let stdout = successful_stdout(
        fixture
            .command()
            .args(["conversation", "delete", &id])
            .output()
            .unwrap(),
    );
    let trash_path = fixture
        .data
        .path()
        .join("trash")
        .join(format!("{id}.jsonl"));
    assert_eq!(
        stdout,
        format!(
            "Deleted conversation: {id}\nHistory recoverable at: {}\n",
            trash_path.display()
        )
    );
    assert!(trash_path.is_file());
    assert_eq!(store.list().unwrap().len(), 1);
    let output = fixture
        .command()
        .args(["conversation", "show", &id])
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("conversation not found"));
}

#[test]
fn each_new_ask_saves_a_distinct_conversation_and_prints_metadata_after_answer() {
    let fixture = CliFixture::new();
    fixture.install_agent();
    let server = FakeDeepSeek::new();
    let first = successful_stdout(
        fixture
            .ask_command(&server)
            .args(["ask", "hello atlas"])
            .output()
            .unwrap(),
    );
    let saved = fixture.store().list().unwrap().remove(0);
    assert_eq!(
        first,
        format!(
            "Hello from Atlas\nConversation: {} | Atlas greeting\n",
            saved.id
        )
    );
    assert!(saved.title_attempted);
    let messages = fixture.store().load(&saved.id).unwrap().messages;
    assert_eq!(messages.len(), 2);
    assert_eq!(messages[0].role, MessageRole::User);
    assert_eq!(messages[0].content, "hello atlas");
    assert_eq!(messages[1].role, MessageRole::Assistant);
    assert_eq!(messages[1].content, "Hello from Atlas");
    let answer_request = server.request();
    assert_eq!(answer_request["stream"], true);
    assert_eq!(
        answer_request["response_format"],
        json!({"type":"json_object"})
    );
    assert_eq!(answer_request["messages"].as_array().unwrap().len(), 2);
    assert!(answer_request["messages"][0]["content"]
        .as_str()
        .unwrap()
        .contains("read_file"));
    assert_eq!(
        answer_request["messages"][1],
        json!({"role":"user","content":"hello atlas"})
    );
    let title_request = server.request();
    assert_eq!(title_request["stream"], false);
    assert!(title_request.get("response_format").is_none());
    assert_eq!(title_request["messages"].as_array().unwrap().len(), 2);
    assert_eq!(
        title_request["messages"][1],
        json!({"role":"user","content":"hello atlas"})
    );
    let second = successful_stdout(
        fixture
            .ask_command(&server)
            .args(["ask", "--stream", "another question"])
            .output()
            .unwrap(),
    );
    let all = fixture.store().list().unwrap();
    assert_eq!(all.len(), 2);
    let newly_saved = all
        .iter()
        .find(|entry| entry.id != saved.id)
        .expect("second ask must create a distinct conversation");
    assert_eq!(
        second,
        format!(
            "Hello from Atlas\nConversation: {} | Atlas greeting\n",
            newly_saved.id
        )
    );
    assert_eq!(server.request()["messages"].as_array().unwrap().len(), 2);
    assert_eq!(server.request()["stream"], false);
}

#[test]
fn cli_marks_restarted_streams_and_saves_only_the_repaired_answer() {
    let fixture = CliFixture::new();
    fixture.install_agent();
    let server = FakeDeepSeek::new();
    let output = fixture
        .ask_command(&server)
        .args(["ask", "malformed streamed reply"])
        .output()
        .unwrap();
    let stdout = successful_stdout(output);
    assert!(
        stdout.contains(
            "Discard me\n[Atlas is correcting its response; previous partial answer discarded.]"
        ),
        "{stdout}"
    );
    assert!(stdout.contains("Hello from Atlas"));
    assert!(!stdout.contains("Discard meHello from Atlas"));
    let saved = fixture.store().list().unwrap();
    let conversation = fixture.store().load(&saved[0].id).unwrap();
    assert_eq!(
        conversation.messages.last().unwrap().content,
        "Hello from Atlas"
    );
    let log = std::fs::read_to_string(fixture.data.path().join("logs/atlas.jsonl")).unwrap();
    assert!(log.contains("protocol.repair"));
    assert!(!log.contains("Discard me"));
    assert!(!log.contains("\"event\":\"operation.failed\""));
}

#[test]
fn cli_recovers_empty_deepseek_stream_after_a_command_without_running_it_twice() {
    let fixture = CliFixture::new();
    fixture.install_agent();
    let server = FakeDeepSeek::new();
    let mut command = fixture.ask_command(&server);
    command.args(["ask", "recover incomplete generation"]);
    let output = output_with_stdin(command, "y\n");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stdout).contains("Hello from Atlas"));
    assert_eq!(
        String::from_utf8_lossy(&output.stderr)
            .matches("Progress: I will run the command once.")
            .count(),
        1
    );
    let content = std::fs::read_to_string(fixture.workspace.path().join("counter.txt")).unwrap();
    assert_eq!(content.lines().count(), 1);
    let _first = server.request();
    let continuation = server.request();
    let repair_request = server.request();
    let pending = continuation["messages"].as_array().unwrap().last().unwrap()["content"]
        .as_str()
        .unwrap();
    let repair: Value = serde_json::from_str(
        repair_request["messages"]
            .as_array()
            .unwrap()
            .last()
            .unwrap()["content"]
            .as_str()
            .unwrap(),
    )
    .unwrap();
    assert_eq!(repair["pending_request"], pending);
    assert_eq!(
        repair["validation_error"],
        "DeepSeek returned an empty answer"
    );
    let log = std::fs::read_to_string(fixture.data.path().join("logs/atlas.jsonl")).unwrap();
    let events: Vec<Value> = log
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(
        events
            .iter()
            .filter(|e| e["event"] == "tool.started" && e["fields"]["tool"] == "exec_command")
            .count(),
        1
    );
    assert!(events.iter().any(|e| e["event"] == "model.stream_ended"
        && e["fields"]["finishReason"] == "stop"
        && e["fields"]["contentBytes"] == 0
        && e["fields"]["doneReceived"] == true));
    assert!(events
        .iter()
        .any(|e| e["event"] == "operation.succeeded" && e["fields"]["operation"] == "cli.command"));
    assert!(!log.contains("I will run the command once."));
    assert!(!log.contains("counter.txt"));
    assert!(!log.contains("fake_key_for_test"));
}

#[test]
fn continuation_uses_supplied_id_and_configured_recent_history_without_retitle() {
    let fixture = CliFixture::new();
    fixture.install_agent();
    let server = FakeDeepSeek::new();
    let store = fixture.store();
    let saved = store.create_turn("Old question", "Old answer").unwrap();
    store
        .append_turn(&saved.id, "Recent question", "Recent answer")
        .unwrap();
    store.rename(&saved.id, "Existing title").unwrap();
    let id = saved.id.to_string();
    let stdout = successful_stdout(
        fixture
            .ask_command(&server)
            .env("ATLAS_HISTORY_MESSAGES", "2")
            .args(["ask", "--conversation", &id, "--stream", "Continue please"])
            .output()
            .unwrap(),
    );
    assert_eq!(
        stdout,
        format!("Hello from Atlas\nConversation: {id} | Existing title\n")
    );
    let request = server.request();
    assert_eq!(request["stream"], true);
    assert_eq!(
        &request["messages"].as_array().unwrap()[1..],
        &[
            json!({"role":"user","content":"Recent question"}),
            json!({"role":"assistant","content":"Recent answer"}),
            json!({"role":"user","content":"Continue please"}),
        ]
    );
    assert!(server.requests.try_recv().is_err());
    assert_eq!(store.list().unwrap().len(), 1);
    let conversation = store.load(&saved.id).unwrap();
    assert_eq!(conversation.metadata.message_count, 6);
    assert_eq!(conversation.messages[4].content, "Continue please");
    assert_eq!(conversation.messages[5].content, "Hello from Atlas");
}

#[test]
fn ask_uses_saved_provider_full_history_and_current_directory_workspace() {
    let fixture = CliFixture::new();
    fixture.install_agent();
    let server = FakeDeepSeek::new();
    let mut settings = AtlasSettings::default();
    settings.deepseek.api_key = Some("fake_key_for_test".to_string());
    settings.deepseek.base_url = server.url.clone();
    settings.conversation.history_limit = 2;
    fixture.settings_store().save(&settings).unwrap();
    let saved = fixture
        .store()
        .create_turn("Old question", "Old answer")
        .unwrap();
    fixture
        .store()
        .append_turn(&saved.id, "Recent question", "Recent answer")
        .unwrap();
    fixture.store().rename(&saved.id, "Existing title").unwrap();

    let stdout = successful_stdout(
        fixture
            .command()
            .args([
                "ask",
                "--conversation",
                &saved.id.to_string(),
                "report workspace",
            ])
            .output()
            .unwrap(),
    );

    assert!(
        stdout.contains(&format!(
            "Workspace: {}",
            fixture.workspace.path().display()
        )),
        "{stdout}"
    );
    let first_request = server.request();
    assert_eq!(
        &first_request["messages"].as_array().unwrap()[1..],
        &[
            json!({"role":"user","content":"Old question"}),
            json!({"role":"assistant","content":"Old answer"}),
            json!({"role":"user","content":"Recent question"}),
            json!({"role":"assistant","content":"Recent answer"}),
            json!({"role":"user","content":"report workspace"}),
        ]
    );
    let continuation = server.request();
    assert!(
        continuation["messages"].as_array().unwrap().last().unwrap()["content"]
            .as_str()
            .unwrap()
            .contains(&fixture.workspace.path().display().to_string())
    );
}

#[test]
fn invalid_history_limit_is_reported_without_sending_a_request() {
    let fixture = CliFixture::new();
    fixture.install_agent();
    let server = FakeDeepSeek::new();
    let output = fixture
        .ask_command(&server)
        .env("ATLAS_HISTORY_MESSAGES", "0")
        .args(["ask", "hello"])
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr)
        .contains("ATLAS_HISTORY_MESSAGES must be a positive integer"));
    assert!(server.requests.try_recv().is_err());
    assert!(fixture.store().list().unwrap().is_empty());
}

#[test]
fn ask_command_loads_deepseek_key_from_dotenv() {
    let fixture = CliFixture::new();
    fixture.install_agent();
    let server = FakeDeepSeek::new();
    std::fs::write(
        fixture.workspace.path().join(".env"),
        format!(
            "DEEPSEEK_API_KEY=fake_key_for_test\nDEEPSEEK_BASE_URL={}\n",
            server.url
        ),
    )
    .unwrap();
    let stdout = successful_stdout(
        fixture
            .command()
            .env_remove("ATLAS_SKIP_DOTENV")
            .args(["ask", "hello atlas"])
            .output()
            .unwrap(),
    );
    assert!(stdout.starts_with("Hello from Atlas\nConversation: "));
    assert_eq!(server.request()["stream"], true);
    assert_eq!(server.request()["stream"], false);
}

#[test]
fn cli_prompts_for_risky_tools_and_full_access_bypasses_the_prompt() {
    let fixture = CliFixture::new();
    fixture.install_agent();
    let server = FakeDeepSeek::new();
    let mut ask = fixture.ask_command(&server);
    ask.args(["ask", "create approved file"]);

    let output = output_with_stdin(ask, "maybe\ny\n");

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(stderr.contains("Allow create_file?"), "{stderr}");
    assert!(stderr.contains("Please answer y or n."), "{stderr}");
    assert_eq!(
        std::fs::read_to_string(fixture.workspace.path().join("approved.txt")).unwrap(),
        "approved"
    );

    let full = CliFixture::new();
    full.install_agent();
    let full_server = FakeDeepSeek::new();
    let mut settings = AtlasSettings::default();
    settings.permissions.default_mode = PermissionMode::FullAccess;
    full.settings_store().save(&settings).unwrap();
    let output = full
        .ask_command(&full_server)
        .args(["ask", "create approved file"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr
            .lines()
            .all(|line| line.starts_with("Context: ") || line.starts_with("Compacting context: ")),
        "{stderr}"
    );
    assert!(full.workspace.path().join("approved.txt").is_file());
}

#[test]
fn cli_denies_on_eof_without_executing_the_tool() {
    let fixture = CliFixture::new();
    fixture.install_agent();
    let server = FakeDeepSeek::new();
    let output = fixture
        .ask_command(&server)
        .args(["ask", "create approved file"])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stderr).contains("Allow create_file?"));
    assert!(!fixture.workspace.path().join("approved.txt").exists());
}

struct FakeDeepSeek {
    url: String,
    requests: Receiver<Value>,
    stopped: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
}

impl FakeDeepSeek {
    fn new() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let (sender, requests) = mpsc::channel();
        let stopped = Arc::new(AtomicBool::new(false));
        let stop = Arc::clone(&stopped);
        let worker = thread::spawn(move || {
            while !stop.load(Ordering::Relaxed) {
                let (mut socket, _) = match listener.accept() {
                    Ok(connection) => connection,
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(5));
                        continue;
                    }
                    Err(error) => panic!("accept failed: {error}"),
                };
                socket
                    .set_read_timeout(Some(Duration::from_secs(3)))
                    .unwrap();
                socket
                    .set_write_timeout(Some(Duration::from_secs(3)))
                    .unwrap();
                let request = read_request(&mut socket);
                if request["models"] == true {
                    let body = r#"{"data":[]}"#;
                    write!(socket, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
                    continue;
                }
                let last_user_content = request["messages"]
                    .as_array()
                    .and_then(|messages| messages.last())
                    .and_then(|message| message["content"].as_str())
                    .unwrap_or_default();
                let streamed_content = if last_user_content == "malformed streamed reply" {
                    r#"{"type":"final","content":"Discard me""#.to_string()
                } else if last_user_content == "recover incomplete generation" {
                    r#"{"type":"tool_call","message":"I will run the command once.","tool":"exec_command","args":{"command":"echo marker >> counter.txt"}}"#.to_string()
                } else if last_user_content == "report workspace" {
                    r#"{"type":"tool_call","tool":"pwd","args":{}}"#.to_string()
                } else if last_user_content == "create approved file" {
                    r#"{"type":"tool_call","tool":"create_file","args":{"path":"approved.txt","content":"approved"}}"#.to_string()
                } else if last_user_content.contains("\"type\": \"tool_results\"") {
                    let continuation: Value = serde_json::from_str(last_user_content).unwrap();
                    if continuation["original_user_request"] == "recover incomplete generation" {
                        String::new()
                    } else {
                        let workspace = continuation["tool_results"][0]["output"].as_str().unwrap();
                        format!(
                            r#"{{"type":"final","content":{}}}"#,
                            serde_json::to_string(workspace).unwrap()
                        )
                    }
                } else {
                    r#"{"type":"final","content":"Hello from Atlas"}"#.to_string()
                };
                let (content_type, body) = if request["stream"] == true {
                    let delta = json!({"choices":[{"delta":{"content":streamed_content},"finish_reason":"stop"}]});
                    (
                        "text/event-stream",
                        format!("data: {delta}\n\ndata: [DONE]\n\n"),
                    )
                } else {
                    ("application/json", json!({"choices":[{"message":{"role":"assistant","content":"Atlas greeting"},"finish_reason":"stop"}]}).to_string())
                };
                sender.send(request).unwrap();
                write!(socket, "HTTP/1.1 200 OK\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
            }
        });
        Self {
            url,
            requests,
            stopped,
            worker: Some(worker),
        }
    }

    fn request(&self) -> Value {
        self.requests.recv_timeout(Duration::from_secs(3)).unwrap()
    }
}

impl Drop for FakeDeepSeek {
    fn drop(&mut self) {
        self.stopped.store(true, Ordering::Relaxed);
        if let Some(worker) = self.worker.take() {
            let result = worker.join();
            if !thread::panicking() {
                result.unwrap();
            }
        }
    }
}

fn read_request(socket: &mut TcpStream) -> Value {
    let mut reader = BufReader::new(socket);
    let mut line = String::new();
    reader.read_line(&mut line).unwrap();
    let models = line == "GET /models HTTP/1.1\r\n";
    if !models {
        assert_eq!(line, "POST /chat/completions HTTP/1.1\r\n");
    }
    let mut length = None;
    let mut authorization = None;
    loop {
        line.clear();
        assert_ne!(reader.read_line(&mut line).unwrap(), 0);
        if line == "\r\n" {
            break;
        }
        let (name, value) = line.split_once(':').unwrap();
        if name.eq_ignore_ascii_case("content-length") {
            length = Some(value.trim().parse::<usize>().unwrap());
        }
        if name.eq_ignore_ascii_case("authorization") {
            authorization = Some(value.trim().to_string());
        }
    }
    assert_eq!(authorization.as_deref(), Some("Bearer fake_key_for_test"));
    if models {
        return json!({"models":true});
    }
    let mut body = vec![0; length.unwrap()];
    reader.read_exact(&mut body).unwrap();
    serde_json::from_slice(&body).unwrap()
}

#[test]
fn cli_reports_when_automatic_compaction_capacity_is_unknown() {
    let fixture = CliFixture::new();
    fixture.install_agent();
    let server = FakeDeepSeek::new();
    let output = fixture
        .ask_command(&server)
        .args(["ask", "hello"])
        .output()
        .unwrap();
    assert!(output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("Automatic compaction unavailable"));
}

#[test]
fn cli_resolves_provider_and_model_fallback_from_the_agent_before_starting_runtime() {
    let fixture = CliFixture::new();
    fixture.install_agent();
    let path = fixture.workspace.path().join("agents/atlas.toml");
    let agent = std::fs::read_to_string(&path).unwrap().replace(
        "model_provider = \"deepseek\"",
        "model_provider = \"openai-compatible\"",
    );
    std::fs::write(path, agent).unwrap();
    let server = FakeDeepSeek::new();
    let output = fixture
        .command()
        .env("ATLAS_COMPATIBLE_API_KEY", "fake_key_for_test")
        .env("ATLAS_COMPATIBLE_BASE_URL", &server.url)
        .args(["ask", "hello atlas"])
        .output()
        .unwrap();
    let stdout = successful_stdout(output);
    assert!(stdout.contains("Hello from Atlas"));
    assert!(stdout.contains("Atlas greeting"));
}
