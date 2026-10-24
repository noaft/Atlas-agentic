//! Explicit browser authorization for remote MCP connections.
mod callback;
mod discovery;
#[cfg(test)]
mod tests;
mod tokens;

use super::{McpConfig, ServerConfig};
use reqwest::{blocking::Client, Url};
use serde::Serialize;
use std::{
    collections::HashMap,
    io::Read,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex, OnceLock,
    },
    time::{Duration, Instant},
};
use tokens::{KeyringStore, Token, TokenStore};

const LOGIN_TIMEOUT: Duration = Duration::from_secs(180);

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectionStatus {
    pub name: String,
    pub transport: String,
    pub oauth: bool,
    pub connected: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

struct PreparedLogin {
    client: Client,
    callback: callback::Callback,
    proof: callback::Proof,
    metadata: discovery::AuthorizationMetadata,
    client_id: String,
    redirect_uri: String,
    scopes: Vec<String>,
    generation: u64,
}

struct Service<S> {
    store: S,
    attempts: Mutex<HashMap<String, Arc<AtomicBool>>>,
    credentials: Mutex<()>,
}

impl<S: TokenStore> Service<S> {
    fn new(store: S) -> Self {
        Self {
            store,
            attempts: Mutex::new(HashMap::new()),
            credentials: Mutex::new(()),
        }
    }

    fn key(&self, config: &ServerConfig) -> Result<String, String> {
        let resource = resource(config)?;
        let oauth = config
            .oauth
            .as_ref()
            .ok_or("this MCP server does not use OAuth")?;
        Ok(tokens::identity(resource.as_str(), &oauth.client_id))
    }

    fn status(&self, name: &str, config: &ServerConfig) -> Result<ConnectionStatus, String> {
        let (connected, error) = if config.oauth.is_some() {
            match self.store.load(&self.key(config)?) {
                Ok(token) => (
                    token.is_some_and(|token| !token.expired() || token.refresh_token.is_some()),
                    None,
                ),
                Err(error) => (false, Some(error)),
            }
        } else {
            (true, None)
        };
        Ok(ConnectionStatus {
            name: name.into(),
            transport: if config.url.is_some() {
                "http"
            } else {
                "stdio"
            }
            .into(),
            oauth: config.oauth.is_some(),
            connected,
            error,
        })
    }

    fn cancel(&self, config: &ServerConfig) -> Result<(), String> {
        if let Some(cancel) = self
            .attempts
            .lock()
            .map_err(|_| "MCP sign-in state unavailable")?
            .get(&self.key(config)?)
        {
            cancel.store(true, Ordering::SeqCst);
        }
        Ok(())
    }

    fn disconnect(&self, config: &ServerConfig) -> Result<(), String> {
        self.cancel(config)?;
        let key = self.key(config)?;
        let _guard = self
            .credentials
            .lock()
            .map_err(|_| "MCP credentials unavailable")?;
        let lease = self.store.lease(&key)?;
        tokens::invalidate(&lease)?;
        self.store.delete(&key)
    }

    fn connect(
        &self,
        config: &ServerConfig,
        open: impl FnOnce(&str) -> Result<(), String>,
        timeout: Duration,
    ) -> Result<(), String> {
        self.connect_with_cancel(config, open, timeout, Arc::new(AtomicBool::new(false)))
    }

    fn connect_with_cancel(
        &self,
        config: &ServerConfig,
        open: impl FnOnce(&str) -> Result<(), String>,
        timeout: Duration,
        canceled: Arc<AtomicBool>,
    ) -> Result<(), String> {
        let deadline = Instant::now() + timeout;
        check_pending(&canceled, deadline)?;
        let key = self.key(config)?;
        {
            let mut attempts = self
                .attempts
                .lock()
                .map_err(|_| "MCP sign-in state unavailable")?;
            if attempts.contains_key(&key) {
                return Err("a sign-in is already pending for this MCP server".into());
            }
            attempts.insert(key.clone(), canceled.clone());
        }
        let _attempt = Attempt {
            attempts: &self.attempts,
            key: key.clone(),
        };
        let generation = {
            let _guard = self
                .credentials
                .lock()
                .map_err(|_| "MCP credentials unavailable")?;
            let lease = self.store.lease(&key)?;
            tokens::generation(&lease)?
        };
        let oauth = config
            .oauth
            .as_ref()
            .ok_or("this MCP server does not use OAuth")?;
        let resource = resource(config)?;
        let client = http_client(config)?;
        let callback = callback::Callback::bind(oauth.redirect_port)?;
        let redirect_uri = callback.redirect_uri()?;
        let proof = callback::Proof::new();
        let discovery_client = client.clone();
        let metadata = cancellable(&canceled, deadline, move || {
            discovery::discover(&resource, &discovery_client)
        })?;
        let (client_id, metadata) = match &oauth.client_id {
            Some(id) => (id.clone(), metadata),
            None => {
                let registration_client = client.clone();
                let uri = redirect_uri.clone();
                cancellable(&canceled, deadline, move || {
                    let id = discovery::register(&registration_client, &metadata, &uri)?;
                    Ok((id, metadata))
                })?
            }
        };
        let scopes = if oauth.scopes.is_empty() {
            metadata.scopes.clone()
        } else {
            oauth.scopes.clone()
        };
        self.finish_connect(
            PreparedLogin {
                client,
                callback,
                proof,
                metadata,
                client_id,
                redirect_uri,
                scopes,
                generation,
            },
            open,
            &canceled,
            deadline,
            &key,
        )
    }

    fn finish_connect(
        &self,
        login: PreparedLogin,
        open: impl FnOnce(&str) -> Result<(), String>,
        canceled: &AtomicBool,
        deadline: Instant,
        key: &str,
    ) -> Result<(), String> {
        let PreparedLogin {
            client,
            mut callback,
            proof,
            metadata,
            client_id,
            redirect_uri,
            scopes,
            generation,
        } = login;
        callback.expect_issuer(&metadata.issuer);
        let mut authorization = metadata.authorization_endpoint.clone();
        if authorization.query_pairs().any(|(key, _)| {
            matches!(
                key.as_ref(),
                "state"
                    | "redirect_uri"
                    | "client_id"
                    | "response_type"
                    | "code_challenge"
                    | "code_challenge_method"
                    | "resource"
                    | "scope"
            )
        }) {
            return Err("OAuth authorization endpoint contains conflicting parameters".into());
        }
        authorization
            .query_pairs_mut()
            .append_pair("response_type", "code")
            .append_pair("client_id", &client_id)
            .append_pair("redirect_uri", &redirect_uri)
            .append_pair("state", &proof.state)
            .append_pair("code_challenge", &proof.challenge)
            .append_pair("code_challenge_method", "S256")
            .append_pair("resource", &metadata.resource);
        if !scopes.is_empty() {
            authorization
                .query_pairs_mut()
                .append_pair("scope", &scopes.join(" "));
        }
        check_pending(canceled, deadline)?;
        open(authorization.as_str())?;
        let code = callback.wait(
            &proof.state,
            canceled,
            deadline.saturating_duration_since(Instant::now()),
        )?;
        check_pending(canceled, deadline)?;
        let fields = vec![
            ("grant_type".into(), "authorization_code".into()),
            ("code".into(), code),
            ("client_id".into(), client_id.clone()),
            ("redirect_uri".into(), redirect_uri),
            ("code_verifier".into(), proof.verifier),
            ("resource".into(), metadata.resource.clone()),
        ];
        let token = cancellable(canceled, deadline, move || {
            exchange(
                &client,
                &metadata.token_endpoint,
                fields,
                &metadata.resource,
                &client_id,
            )
        })?;
        let _guard = self
            .credentials
            .lock()
            .map_err(|_| "MCP credentials unavailable")?;
        let lease = self.store.lease(key)?;
        check_pending(canceled, deadline)?;
        if tokens::generation(&lease)? != generation {
            return Err("MCP sign-in canceled by a disconnection".into());
        }
        self.store.save(key, &token)?;
        if let Err(error) = check_pending(canceled, deadline) {
            self.store.delete(key)?;
            return Err(error);
        }
        Ok(())
    }

    fn access(&self, config: &ServerConfig) -> Result<Option<String>, String> {
        if config.oauth.is_none() {
            return Ok(None);
        }
        let key = self.key(config)?;
        let _guard = self
            .credentials
            .lock()
            .map_err(|_| "MCP credentials unavailable")?;
        let _lease = self.store.lease(&key)?;
        let Some(token) = self.store.load(&key)? else {
            return Err("MCP sign-in required; use Connect in Settings".into());
        };
        if token.resource != resource(config)?.as_str()
            || config
                .oauth
                .as_ref()
                .and_then(|oauth| oauth.client_id.as_ref())
                .is_some_and(|id| *id != token.client_id)
        {
            return Err(
                "saved MCP credentials do not match this server; reconnect in Settings".into(),
            );
        }
        if !token.expired() {
            return Ok(Some(token.access_token));
        }
        let Some(refresh) = &token.refresh_token else {
            return Err("MCP sign-in expired; reconnect in Settings".into());
        };
        let endpoint = discovery::endpoint(&token.token_endpoint)?;
        let fields = vec![
            ("grant_type".into(), "refresh_token".into()),
            ("refresh_token".into(), refresh.clone()),
            ("client_id".into(), token.client_id.clone()),
            ("resource".into(), token.resource.clone()),
        ];
        let refreshed = exchange(
            &http_client(config)?,
            &endpoint,
            fields,
            &token.resource,
            &token.client_id,
        );
        let mut refreshed = match refreshed {
            Ok(token) => token,
            Err(_) => {
                self.store.delete(&key)?;
                return Err("MCP token refresh failed; reconnect in Settings".into());
            }
        };
        if refreshed.refresh_token.is_none() {
            refreshed.refresh_token = token.refresh_token;
        }
        if self.store.save(&key, &refreshed).is_err() {
            let _ = self.store.delete(&key);
            return Err(
                "could not securely save refreshed MCP credentials; reconnect in Settings".into(),
            );
        }
        Ok(Some(refreshed.access_token))
    }
}

struct Attempt<'a> {
    attempts: &'a Mutex<HashMap<String, Arc<AtomicBool>>>,
    key: String,
}
impl Drop for Attempt<'_> {
    fn drop(&mut self) {
        if let Ok(mut attempts) = self.attempts.lock() {
            attempts.remove(&self.key);
        }
    }
}

fn check_pending(canceled: &AtomicBool, deadline: Instant) -> Result<(), String> {
    if canceled.load(Ordering::SeqCst) {
        Err("MCP sign-in canceled".into())
    } else if Instant::now() >= deadline {
        Err("MCP sign-in timed out".into())
    } else {
        Ok(())
    }
}

fn cancellable<T: Send + 'static>(
    canceled: &AtomicBool,
    deadline: Instant,
    work: impl FnOnce() -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    check_pending(canceled, deadline)?;
    let (sender, receiver) = std::sync::mpsc::channel();
    std::thread::Builder::new()
        .name("atlas-mcp-oauth".into())
        .spawn(move || {
            let _ = sender.send(work());
        })
        .map_err(|_| "could not start MCP sign-in worker")?;
    loop {
        check_pending(canceled, deadline)?;
        match receiver.recv_timeout(Duration::from_millis(25)) {
            Ok(result) => {
                check_pending(canceled, deadline)?;
                return result;
            }
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
            Err(_) => return Err("MCP sign-in worker stopped".into()),
        }
    }
}

fn resource(config: &ServerConfig) -> Result<Url, String> {
    super::config::validate_http_url(
        config
            .url
            .as_deref()
            .ok_or("OAuth requires an HTTP MCP server")?,
    )
}

fn http_client(config: &ServerConfig) -> Result<Client, String> {
    Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .retry(reqwest::retry::never())
        .timeout(Duration::from_millis(config.timeout_ms.min(30_000)))
        .build()
        .map_err(|_| "could not create OAuth HTTP client".into())
}

fn exchange(
    client: &Client,
    endpoint: &Url,
    fields: Vec<(String, String)>,
    resource: &str,
    client_id: &str,
) -> Result<Token, String> {
    let response = client
        .post(endpoint.clone())
        .form(&fields)
        .send()
        .map_err(|_| "OAuth token exchange failed")?;
    if !response.status().is_success() {
        return Err("OAuth token exchange was rejected; reconnect in Settings".into());
    }
    let mut body = Vec::new();
    response
        .take(1024 * 1024 + 1)
        .read_to_end(&mut body)
        .map_err(|_| "could not read OAuth token response")?;
    if body.len() > 1024 * 1024 {
        return Err("OAuth token response exceeds the size limit".into());
    }
    let response = serde_json::from_slice(&body).map_err(|_| "invalid OAuth token response")?;
    Token::from_response(response, resource, client_id, endpoint.as_str())
}

fn service() -> &'static Service<KeyringStore> {
    static SERVICE: OnceLock<Service<KeyringStore>> = OnceLock::new();
    SERVICE.get_or_init(|| Service::new(KeyringStore))
}

fn configured(server: &str) -> Result<ServerConfig, String> {
    McpConfig::from_environment()?
        .servers
        .remove(server)
        .ok_or_else(|| "unknown configured MCP server".into())
}

pub fn list_connections() -> Result<Vec<ConnectionStatus>, String> {
    McpConfig::from_environment()?
        .servers
        .iter()
        .map(|(name, config)| service().status(name, config))
        .collect()
}

pub fn connect_server(server: &str) -> Result<(), String> {
    service().connect(&configured(server)?, open_browser, LOGIN_TIMEOUT)
}

/// Native command dispatch can reserve cancellation before scheduling blocking work.
pub fn connect_server_with_cancel(server: &str, canceled: Arc<AtomicBool>) -> Result<(), String> {
    check_pending(&canceled, Instant::now() + LOGIN_TIMEOUT)?;
    service().connect_with_cancel(&configured(server)?, open_browser, LOGIN_TIMEOUT, canceled)
}

pub fn disconnect_server(server: &str) -> Result<(), String> {
    service().disconnect(&configured(server)?)
}
pub fn cancel_connection(server: &str) -> Result<(), String> {
    service().cancel(&configured(server)?)
}
pub fn access_token(config: &ServerConfig) -> Result<Option<String>, String> {
    service().access(config)
}

fn open_browser(url: &str) -> Result<(), String> {
    #[cfg(windows)]
    {
        crate::tools::system_launcher()
            .launch_file(crate::tools::Platform::Windows, url)
            .map_err(|_| "could not open the default browser".into())
    }
    #[cfg(not(windows))]
    {
        let program = if cfg!(target_os = "macos") {
            "open"
        } else {
            "xdg-open"
        };
        let mut child = std::process::Command::new(program)
            .arg(url)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .map_err(|_| {
                "could not open the default browser; configure a browser before connecting"
            })?;
        std::thread::spawn(move || {
            let _ = child.wait();
        });
        Ok(())
    }
}
