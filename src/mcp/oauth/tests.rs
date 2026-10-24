use super::*;
use base64::Engine;
use serde_json::json;
use sha2::Digest;
use std::{
    collections::HashMap,
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    sync::{Arc, Mutex},
    time::Duration,
};
use tokens::{Token, TokenStore};

#[derive(Default)]
struct MemoryStore(Mutex<HashMap<String, Token>>);
impl TokenStore for MemoryStore {
    fn load(&self, key: &str) -> Result<Option<Token>, String> {
        Ok(self.0.lock().unwrap().get(key).cloned())
    }
    fn save(&self, key: &str, token: &Token) -> Result<(), String> {
        self.0.lock().unwrap().insert(key.into(), token.clone());
        Ok(())
    }
    fn delete(&self, key: &str) -> Result<(), String> {
        self.0.lock().unwrap().remove(key);
        Ok(())
    }
}

fn fixture(
    requests: usize,
) -> (
    ServerConfig,
    Arc<Mutex<String>>,
    std::thread::JoinHandle<()>,
) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let challenge = Arc::new(Mutex::new(String::new()));
    let expected_challenge = challenge.clone();
    let fixture_base = base.clone();
    let worker = std::thread::spawn(move || {
        for index in 0..requests {
            let (mut socket, _) = listener.accept().unwrap();
            socket
                .set_read_timeout(Some(Duration::from_secs(3)))
                .unwrap();
            let mut bytes = Vec::new();
            let header_end;
            loop {
                let mut byte = [0];
                socket.read_exact(&mut byte).unwrap();
                bytes.push(byte[0]);
                if bytes.ends_with(b"\r\n\r\n") {
                    header_end = bytes.len();
                    break;
                }
            }
            let headers = String::from_utf8(bytes.clone()).unwrap();
            let count: usize = headers
                .lines()
                .find_map(|line| {
                    line.to_ascii_lowercase()
                        .strip_prefix("content-length:")
                        .map(|value| value.trim().parse().unwrap())
                })
                .unwrap_or(0);
            let mut body = vec![0; count];
            socket.read_exact(&mut body).unwrap();
            bytes.extend(body);
            let (status, body, extra) = match index {
                0 => {
                    assert!(headers.starts_with("GET /mcp "));
                    (401,json!({}),format!("WWW-Authenticate: Bearer resource_metadata=\"{fixture_base}/resource\", scope=\"files:read\"\r\n"))
                }
                1 => {
                    assert!(headers.starts_with("GET /resource "));
                    (
                        200,
                        json!({"resource":format!("{fixture_base}/mcp"),"authorization_servers":[fixture_base],"scopes_supported":["files:read"]}),
                        String::new(),
                    )
                }
                2 => {
                    assert!(headers.starts_with("GET /.well-known/oauth-authorization-server "));
                    (
                        200,
                        json!({"issuer":fixture_base,"authorization_endpoint":format!("{fixture_base}/authorize"),"token_endpoint":format!("{fixture_base}/token"),"code_challenge_methods_supported":["S256"],"token_endpoint_auth_methods_supported":["none"]}),
                        String::new(),
                    )
                }
                _ => {
                    assert!(headers.starts_with("POST /token "));
                    assert!(!headers.to_ascii_lowercase().contains("authorization:"));
                    let body = std::str::from_utf8(&bytes[header_end..]).unwrap();
                    let form_url =
                        reqwest::Url::parse(&format!("http://localhost/?{body}")).unwrap();
                    let form: HashMap<_, _> = form_url.query_pairs().into_owned().collect();
                    assert_eq!(form["resource"], format!("{fixture_base}/mcp"));
                    assert_eq!(form["client_id"], "registered-client");
                    assert!(!form.contains_key("client_secret"));
                    if index == 3 {
                        assert_eq!(form["grant_type"], "authorization_code");
                        assert_eq!(form["code"], "private-code");
                        let challenge = base64::engine::general_purpose::URL_SAFE_NO_PAD
                            .encode(sha2::Sha256::digest(form["code_verifier"].as_bytes()));
                        assert_eq!(challenge, *expected_challenge.lock().unwrap());
                    } else {
                        assert_eq!(form["grant_type"], "refresh_token");
                        assert_eq!(form["refresh_token"], "private-refresh");
                    }
                    (
                        200,
                        json!({"access_token":if index == 3 {"private-access"} else {"rotated-access"},"refresh_token":"rotated-refresh","token_type":"Bearer","expires_in":3600}),
                        String::new(),
                    )
                }
            };
            let body = body.to_string();
            write!(socket,"HTTP/1.1 {status} OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n{extra}Connection: close\r\n\r\n{body}",body.len()).unwrap();
        }
    });
    let config = super::super::McpConfig::from_toml(&format!(
        "[servers.remote]\nurl='{base}/mcp'\n[servers.remote.oauth]\nclient_id='registered-client'"
    ))
    .unwrap()
    .servers
    .remove("remote")
    .unwrap();
    (config, challenge, worker)
}

fn browser(url: &str, challenge: &Mutex<String>) -> Result<(), String> {
    let url = reqwest::Url::parse(url).unwrap();
    let params: HashMap<_, _> = url.query_pairs().into_owned().collect();
    assert_eq!(params["code_challenge_method"], "S256");
    assert_eq!(params["scope"], "files:read");
    assert_eq!(params["response_type"], "code");
    *challenge.lock().unwrap() = params["code_challenge"].clone();
    let mut callback = reqwest::Url::parse(&params["redirect_uri"]).unwrap();
    callback
        .query_pairs_mut()
        .append_pair("state", &params["state"])
        .append_pair("code", "private-code");
    std::thread::spawn(move || {
        let mut socket =
            TcpStream::connect((callback.host_str().unwrap(), callback.port().unwrap())).unwrap();
        write!(
            socket,
            "GET {}?{} HTTP/1.1\r\nHost: localhost\r\n\r\n",
            callback.path(),
            callback.query().unwrap()
        )
        .unwrap();
        let mut response = String::new();
        socket.read_to_string(&mut response).unwrap();
        assert!(!response.contains("private-code"));
    });
    Ok(())
}

#[test]
fn full_sign_in_keeps_tokens_out_of_status_and_disconnect_forgets_them() {
    let (config, challenge, worker) = fixture(4);
    let service = Service::new(MemoryStore::default());
    service
        .connect(
            &config,
            |url| browser(url, &challenge),
            Duration::from_secs(2),
        )
        .unwrap();
    assert_eq!(
        service.access(&config).unwrap().as_deref(),
        Some("private-access")
    );
    let status = service.status("remote", &config).unwrap();
    assert!(status.connected);
    assert!(!serde_json::to_string(&status).unwrap().contains("private"));
    service.disconnect(&config).unwrap();
    assert!(service.access(&config).unwrap_err().contains("Settings"));
    worker.join().unwrap();
}

#[test]
fn canceled_sign_in_never_saves_credentials() {
    let (config, _, worker) = fixture(3);
    let service = Service::new(MemoryStore::default());
    let key = service.key(&config).unwrap();
    let error = service
        .connect(
            &config,
            |_| {
                service.cancel(&config)?;
                Ok(())
            },
            Duration::from_secs(2),
        )
        .unwrap_err();
    assert!(error.contains("canceled"));
    assert!(service.store.load(&key).unwrap().is_none());
    worker.join().unwrap();
}

#[test]
fn expired_tokens_refresh_once_across_concurrent_chat_calls() {
    let (config, challenge, worker) = fixture(5);
    let service = Service::new(MemoryStore::default());
    service
        .connect(
            &config,
            |url| browser(url, &challenge),
            Duration::from_secs(2),
        )
        .unwrap();
    let key = service.key(&config).unwrap();
    let mut token = service.store.load(&key).unwrap().unwrap();
    token.expires_at = Some(0);
    token.refresh_token = Some("private-refresh".into());
    service.store.save(&key, &token).unwrap();
    std::thread::scope(|scope| {
        let one = scope.spawn(|| service.access(&config).unwrap());
        let two = scope.spawn(|| service.access(&config).unwrap());
        assert_eq!(one.join().unwrap().as_deref(), Some("rotated-access"));
        assert_eq!(two.join().unwrap().as_deref(), Some("rotated-access"));
    });
    assert_eq!(
        service
            .store
            .load(&key)
            .unwrap()
            .unwrap()
            .refresh_token
            .as_deref(),
        Some("rotated-refresh")
    );
    worker.join().unwrap();
}

#[test]
fn cancel_during_credential_save_cannot_publish_a_late_connection() {
    struct BlockingStore {
        inner: MemoryStore,
        started: std::sync::mpsc::SyncSender<()>,
        release: Mutex<std::sync::mpsc::Receiver<()>>,
    }
    impl TokenStore for BlockingStore {
        fn load(&self, key: &str) -> Result<Option<Token>, String> {
            self.inner.load(key)
        }
        fn delete(&self, key: &str) -> Result<(), String> {
            self.inner.delete(key)
        }
        fn save(&self, key: &str, token: &Token) -> Result<(), String> {
            self.started.send(()).unwrap();
            self.release
                .lock()
                .unwrap()
                .recv_timeout(Duration::from_secs(3))
                .unwrap();
            self.inner.save(key, token)
        }
    }
    let (config, challenge, worker) = fixture(4);
    let (started_tx, started_rx) = std::sync::mpsc::sync_channel(1);
    let (release_tx, release_rx) = std::sync::mpsc::channel();
    let service = Service::new(BlockingStore {
        inner: MemoryStore::default(),
        started: started_tx,
        release: Mutex::new(release_rx),
    });
    std::thread::scope(|scope| {
        let login = scope.spawn(|| {
            service.connect(
                &config,
                |url| browser(url, &challenge),
                Duration::from_secs(2),
            )
        });
        started_rx.recv_timeout(Duration::from_secs(2)).unwrap();
        service.cancel(&config).unwrap();
        release_tx.send(()).unwrap();
        assert!(login.join().unwrap().unwrap_err().contains("canceled"));
    });
    assert!(service
        .store
        .load(&service.key(&config).unwrap())
        .unwrap()
        .is_none());
    worker.join().unwrap();
}

#[test]
fn disconnect_in_another_service_invalidates_a_pending_sign_in() {
    struct SharedStore {
        data: Arc<MemoryStore>,
        path: std::path::PathBuf,
    }
    impl TokenStore for SharedStore {
        fn load(&self, key: &str) -> Result<Option<Token>, String> {
            self.data.load(key)
        }
        fn save(&self, key: &str, token: &Token) -> Result<(), String> {
            self.data.save(key, token)
        }
        fn delete(&self, key: &str) -> Result<(), String> {
            self.data.delete(key)
        }
        fn lease(&self, _key: &str) -> Result<Option<std::fs::File>, String> {
            let file = std::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .create(true)
                .truncate(false)
                .open(&self.path)
                .unwrap();
            fs2::FileExt::lock_exclusive(&file).unwrap();
            Ok(Some(file))
        }
    }
    let root = tempfile::tempdir().unwrap();
    let data = Arc::new(MemoryStore::default());
    let service = Service::new(SharedStore {
        data: data.clone(),
        path: root.path().join("lock"),
    });
    let other = Service::new(SharedStore {
        data: data.clone(),
        path: root.path().join("lock"),
    });
    let (config, challenge, worker) = fixture(4);
    let result = service.connect(
        &config,
        |url| {
            other.disconnect(&config)?;
            browser(url, &challenge)
        },
        Duration::from_secs(2),
    );
    assert!(result.unwrap_err().contains("canceled"));
    assert!(data.load(&service.key(&config).unwrap()).unwrap().is_none());
    worker.join().unwrap();
}

#[test]
fn rejected_refresh_forgets_credentials_without_exposing_the_response() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let worker = std::thread::spawn(move || {
        let (mut socket, _) = listener.accept().unwrap();
        socket
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        let mut request = [0; 4096];
        assert!(socket.read(&mut request).unwrap() > 0);
        let body = r#"{"error":"invalid_grant","error_description":"private-refresh-details"}"#;
        write!(socket,"HTTP/1.1 400 Bad Request\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len()).unwrap();
    });
    let config = McpConfig::from_toml(&format!(
        "[servers.remote]\nurl='{base}/mcp'\n[servers.remote.oauth]\nclient_id='registered-client'"
    ))
    .unwrap()
    .servers
    .remove("remote")
    .unwrap();
    let service = Service::new(MemoryStore::default());
    let key = service.key(&config).unwrap();
    service
        .store
        .save(
            &key,
            &Token {
                access_token: "expired".into(),
                refresh_token: Some("private-refresh".into()),
                expires_at: Some(0),
                resource: format!("{base}/mcp"),
                client_id: "registered-client".into(),
                token_endpoint: format!("{base}/token"),
            },
        )
        .unwrap();
    let error = service.access(&config).unwrap_err();
    assert!(error.contains("Settings"));
    assert!(!error.contains("private-refresh-details"));
    assert!(service.store.load(&key).unwrap().is_none());
    assert!(service.access(&config).unwrap_err().contains("Settings"));
    worker.join().unwrap();
}

#[test]
fn corrupt_credential_status_keeps_disconnect_recovery_available() {
    struct CorruptStore;
    impl TokenStore for CorruptStore {
        fn load(&self, _: &str) -> Result<Option<Token>, String> {
            Err("invalid saved MCP credentials; disconnect and sign in again".into())
        }
        fn save(&self, _: &str, _: &Token) -> Result<(), String> {
            unreachable!()
        }
        fn delete(&self, _: &str) -> Result<(), String> {
            Ok(())
        }
    }
    let config = McpConfig::from_toml(
        "[servers.remote]\nurl='https://server.example/mcp'\n[servers.remote.oauth]",
    )
    .unwrap()
    .servers
    .remove("remote")
    .unwrap();
    let service = Service::new(CorruptStore);
    let status = service.status("remote", &config).unwrap();
    assert!(!status.connected);
    assert!(status.error.unwrap().contains("disconnect"));
    service.disconnect(&config).unwrap();
}

#[test]
fn cancellation_before_the_worker_starts_never_opens_a_browser() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let config = McpConfig::from_toml(&format!(
        "[servers.remote]\nurl='http://{}/mcp'\ntimeout_ms=1\n[servers.remote.oauth]",
        listener.local_addr().unwrap()
    ))
    .unwrap()
    .servers
    .remove("remote")
    .unwrap();
    let service = Service::new(MemoryStore::default());
    let canceled = Arc::new(std::sync::atomic::AtomicBool::new(true));
    let error = service
        .connect_with_cancel(
            &config,
            |_| panic!("canceled sign-in opened a browser"),
            Duration::from_secs(1),
            canceled,
        )
        .unwrap_err();
    assert!(error.contains("canceled"));
    assert!(service.attempts.lock().unwrap().is_empty());
}
