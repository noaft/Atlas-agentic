use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use sha2::{Digest, Sha256};
use std::{
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, Instant},
};

pub(super) struct Proof {
    pub verifier: String,
    pub challenge: String,
    pub state: String,
}

impl Proof {
    pub fn new() -> Self {
        let random = [
            uuid::Uuid::new_v4(),
            uuid::Uuid::new_v4(),
            uuid::Uuid::new_v4(),
        ];
        let bytes: Vec<u8> = random
            .iter()
            .flat_map(|id| id.as_bytes().iter().copied())
            .collect();
        let verifier = URL_SAFE_NO_PAD.encode(bytes);
        let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
        let state = uuid::Uuid::new_v4().to_string();
        crate::diagnostics::protect_secret(&verifier);
        crate::diagnostics::protect_secret(&state);
        Self {
            verifier,
            challenge,
            state,
        }
    }
}

pub(super) struct Callback {
    listener: TcpListener,
    issuer: Option<String>,
}

impl Callback {
    pub fn bind(port: Option<u16>) -> Result<Self, String> {
        let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, port.unwrap_or(0)))
            .map_err(|_| "could not open the OAuth localhost callback port")?;
        listener
            .set_nonblocking(true)
            .map_err(|_| "could not configure OAuth callback")?;
        Ok(Self {
            listener,
            issuer: None,
        })
    }

    pub fn expect_issuer(&mut self, issuer: &str) {
        self.issuer = Some(issuer.into());
    }

    pub fn redirect_uri(&self) -> Result<String, String> {
        let address = self
            .listener
            .local_addr()
            .map_err(|_| "OAuth callback unavailable")?;
        Ok(format!("http://{address}/callback"))
    }

    pub fn wait(
        self,
        state: &str,
        canceled: &AtomicBool,
        timeout: Duration,
    ) -> Result<String, String> {
        let deadline = Instant::now() + timeout;
        loop {
            if canceled.load(Ordering::SeqCst) {
                return Err("MCP sign-in canceled".into());
            }
            if Instant::now() >= deadline {
                return Err("MCP sign-in timed out".into());
            }
            match self.listener.accept() {
                Ok((mut socket, _)) => {
                    let result = read_callback(
                        &mut socket,
                        state,
                        self.issuer.as_deref(),
                        canceled,
                        deadline,
                    );
                    let status = if matches!(&result, Ok(Some(_))) {
                        "200 OK"
                    } else {
                        "400 Bad Request"
                    };
                    let _ = socket.set_nonblocking(false);
                    let _ = socket.set_write_timeout(Some(Duration::from_millis(250)));
                    let body = "Return to Atlas to check the connection.";
                    let _ = write!(socket, "HTTP/1.1 {status}\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Length: {}\r\nCache-Control: no-store\r\nReferrer-Policy: no-referrer\r\nContent-Security-Policy: default-src 'none'\r\nConnection: close\r\n\r\n{body}", body.len());
                    match result {
                        Ok(Some(code)) => return Ok(code),
                        Err(error) => return Err(error),
                        Ok(None) => {}
                    }
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(Duration::from_millis(20))
                }
                Err(_) => return Err("could not accept OAuth callback".into()),
            }
        }
    }
}

fn read_callback(
    socket: &mut TcpStream,
    expected: &str,
    expected_issuer: Option<&str>,
    canceled: &AtomicBool,
    login_deadline: Instant,
) -> Result<Option<String>, String> {
    socket
        .set_nonblocking(true)
        .map_err(|_| "could not read OAuth callback")?;
    let deadline = login_deadline.min(Instant::now() + Duration::from_secs(2));
    let mut bytes = Vec::new();
    while !bytes.windows(4).any(|part| part == b"\r\n\r\n") {
        if canceled.load(Ordering::SeqCst) {
            return Err("MCP sign-in canceled".into());
        }
        if Instant::now() >= deadline || bytes.len() >= 8192 {
            return Ok(None);
        }
        let mut buffer = [0; 1024];
        match socket.read(&mut buffer) {
            Ok(0) => return Ok(None),
            Ok(count) => bytes.extend_from_slice(&buffer[..count]),
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(10))
            }
            Err(_) => return Ok(None),
        }
    }
    let Ok(request) = std::str::from_utf8(&bytes) else {
        return Ok(None);
    };
    let mut first = request
        .lines()
        .next()
        .unwrap_or_default()
        .split_whitespace();
    if first.next() != Some("GET") {
        return Ok(None);
    }
    let Some(path) = first.next().filter(|path| path.starts_with("/callback?")) else {
        return Ok(None);
    };
    let url = reqwest::Url::parse(&format!("http://127.0.0.1{path}"))
        .map_err(|_| "invalid OAuth callback")?;
    let params: Vec<_> = url.query_pairs().collect();
    let states: Vec<_> = params.iter().filter(|(key, _)| key == "state").collect();
    if states.len() != 1 || states[0].1 != expected {
        return Ok(None);
    }
    let issuers: Vec<_> = params.iter().filter(|(key, _)| key == "iss").collect();
    if let Some(expected) = expected_issuer {
        if !issuers.is_empty()
            && (issuers.len() != 1
                || reqwest::Url::parse(&issuers[0].1).ok() != reqwest::Url::parse(expected).ok())
        {
            return Err("MCP authorization response issuer mismatch".into());
        }
    }
    if params.iter().any(|(key, _)| key == "error") {
        return Err("MCP authorization was declined or failed".into());
    }
    let codes: Vec<_> = params.iter().filter(|(key, _)| key == "code").collect();
    if codes.len() != 1 || codes[0].1.is_empty() || codes[0].1.len() > 4096 {
        return Ok(None);
    }
    let code = codes[0].1.to_string();
    crate::diagnostics::protect_secret(&code);
    Ok(Some(code))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        io::{Read, Write},
        net::TcpStream,
        sync::atomic::AtomicBool,
        time::Duration,
    };

    #[test]
    fn pkce_challenge_uses_s256_and_has_a_fresh_state() {
        let first = Proof::new();
        let second = Proof::new();
        assert_ne!(first.verifier, second.verifier);
        assert_ne!(first.state, second.state);
        assert_eq!(
            first.challenge,
            base64::Engine::encode(
                &base64::engine::general_purpose::URL_SAFE_NO_PAD,
                sha2::Sha256::digest(first.verifier.as_bytes())
            )
        );
        assert!(first.verifier.len() >= 43);
    }

    #[test]
    fn callback_accepts_matching_state_and_never_echoes_code() {
        let callback = Callback::bind(None).unwrap();
        let address = callback.listener.local_addr().unwrap();
        let browser = std::thread::spawn(move || {
            let mut socket = TcpStream::connect(address).unwrap();
            socket.write_all(b"GET /callback?code=private-code&state=expected HTTP/1.1\r\nHost: localhost\r\n\r\n").unwrap();
            let mut response = String::new();
            socket.read_to_string(&mut response).unwrap();
            assert!(!response.contains("private-code"));
        });
        assert_eq!(
            callback
                .wait("expected", &AtomicBool::new(false), Duration::from_secs(1))
                .unwrap(),
            "private-code"
        );
        browser.join().unwrap();
    }

    #[test]
    fn wrong_state_is_rejected_without_consuming_the_pending_login() {
        let callback = Callback::bind(None).unwrap();
        let address = callback.listener.local_addr().unwrap();
        let browser = std::thread::spawn(move || {
            for state in ["wrong", "expected"] {
                let mut socket = TcpStream::connect(address).unwrap();
                write!(socket, "GET /callback?code=private-code&state={state} HTTP/1.1\r\nHost: localhost\r\n\r\n").unwrap();
                let mut response = String::new();
                socket.read_to_string(&mut response).unwrap();
                if state == "wrong" {
                    assert!(response.starts_with("HTTP/1.1 400"));
                }
            }
        });
        assert_eq!(
            callback
                .wait("expected", &AtomicBool::new(false), Duration::from_secs(2))
                .unwrap(),
            "private-code"
        );
        browser.join().unwrap();
    }

    #[test]
    fn canceled_and_expired_logins_return_without_waiting_for_a_browser() {
        let callback = Callback::bind(None).unwrap();
        assert!(callback
            .wait("state", &AtomicBool::new(true), Duration::from_secs(60))
            .unwrap_err()
            .contains("canceled"));
        let callback = Callback::bind(None).unwrap();
        assert!(callback
            .wait("state", &AtomicBool::new(false), Duration::from_millis(20))
            .unwrap_err()
            .contains("timed out"));
    }

    #[test]
    fn mismatched_authorization_response_issuer_is_rejected() {
        let mut callback = Callback::bind(None).unwrap();
        callback.expect_issuer("https://trusted.example");
        let address = callback.listener.local_addr().unwrap();
        let browser = std::thread::spawn(move || {
            let mut socket = TcpStream::connect(address).unwrap();
            socket.write_all(b"GET /callback?code=private-code&state=expected&iss=https%3A%2F%2Fother.example HTTP/1.1\r\nHost: localhost\r\n\r\n").unwrap();
            let mut response = String::new();
            socket.read_to_string(&mut response).unwrap();
            assert!(!response.contains("private-code"));
        });
        assert!(callback
            .wait("expected", &AtomicBool::new(false), Duration::from_secs(2))
            .unwrap_err()
            .contains("issuer"));
        browser.join().unwrap();
    }
}
