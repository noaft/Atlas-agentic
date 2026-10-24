use super::ServerConfig;
use reqwest::{
    blocking::{Client as HttpClient, Response},
    header::{HeaderValue, ACCEPT, CONTENT_TYPE},
    StatusCode,
};
use serde_json::{json, Value};
use std::{
    io::{BufRead, BufReader, Read},
    time::Duration,
};

const MAX_BYTES: usize = 1024 * 1024;
const PROTOCOL_VERSION: &str = "2025-11-25";

/// A single Streamable HTTP session. Never replay POSTs, including authentication failures.
pub(super) struct Client {
    http: HttpClient,
    config: ServerConfig,
    next_id: u64,
    session: Option<HeaderValue>,
    protocol: Option<String>,
}

impl Client {
    pub fn connect(config: &ServerConfig) -> Result<Self, String> {
        let http = HttpClient::builder()
            .redirect(reqwest::redirect::Policy::none())
            .retry(reqwest::retry::never())
            .timeout(Duration::from_millis(config.timeout_ms))
            .build()
            .map_err(|_| "could not create MCP HTTP client")?;
        let mut client = Self {
            http,
            config: config.clone(),
            next_id: 1,
            session: None,
            protocol: None,
        };
        let initialized = client.request("initialize", json!({"protocolVersion":PROTOCOL_VERSION,"capabilities":{},"clientInfo":{"name":"atlas","version":env!("CARGO_PKG_VERSION")}}))?;
        let version = initialized["protocolVersion"]
            .as_str()
            .ok_or("invalid MCP protocol version")?;
        if !matches!(version, "2025-11-25" | "2025-06-18" | "2025-03-26") {
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
        client.protocol = Some(version.into());
        let response =
            client.post(&json!({"jsonrpc":"2.0","method":"notifications/initialized"}))?;
        if response.status() != StatusCode::ACCEPTED && response.status() != StatusCode::NO_CONTENT
        {
            return Err("MCP HTTP server did not acknowledge initialization notification".into());
        }
        Ok(client)
    }

    pub fn request(&mut self, method: &str, params: Value) -> Result<Value, String> {
        let id = self.next_id;
        self.next_id += 1;
        let response =
            self.post(&json!({"jsonrpc":"2.0","id":id,"method":method,"params":params}))?;
        if method == "initialize" {
            if let Some(session) = response.headers().get("mcp-session-id") {
                if session.as_bytes().is_empty()
                    || session.as_bytes().len() > 1024
                    || !session.as_bytes().iter().all(|c| (0x21..=0x7e).contains(c))
                {
                    return Err("invalid MCP HTTP session identifier".into());
                }
                self.session = Some(session.clone());
            }
        }
        read_response(response, &json!(id))
    }

    fn post(&self, message: &Value) -> Result<Response, String> {
        let body = serde_json::to_vec(message).map_err(|_| "could not encode MCP request")?;
        if body.len() > MAX_BYTES {
            return Err("MCP request exceeds the 1 MiB message limit".into());
        }
        let mut request = self
            .http
            .post(
                self.config
                    .url
                    .as_deref()
                    .ok_or("MCP HTTP URL unavailable")?,
            )
            .header(ACCEPT, "application/json, text/event-stream")
            .header(CONTENT_TYPE, "application/json")
            .body(body);
        if let Some(session) = &self.session {
            request = request.header("Mcp-Session-Id", session);
        }
        if let Some(protocol) = &self.protocol {
            request = request.header("MCP-Protocol-Version", protocol);
        }
        if let Some(token) = if self.config.oauth.is_some() {
            super::oauth::access_token(&self.config)?
        } else {
            None
        } {
            request = request.bearer_auth(token);
        }
        let response = request.send().map_err(|error| {
            if error.is_timeout() {
                "MCP HTTP request timed out; action outcome may be unknown"
            } else {
                "MCP HTTP transport failed; action outcome may be unknown"
            }
        })?;
        match response.status() {
            StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN=>Err("MCP authorization required or expired; use Connect in Settings, then explicitly retry if appropriate".into()),
            status if !status.is_success()=>Err("MCP HTTP server returned an unsuccessful status; action outcome may be unknown".into()),
            _=>Ok(response),
        }
    }
}

fn read_response(response: Response, id: &Value) -> Result<Value, String> {
    let content_type = response
        .headers()
        .get(CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.split(';').next())
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();
    match content_type.as_str() {
        "application/json" => {
            let mut body = Vec::new();
            response
                .take((MAX_BYTES + 1) as u64)
                .read_to_end(&mut body)
                .map_err(|_| "could not read MCP HTTP response; action outcome may be unknown")?;
            if body.len() > MAX_BYTES {
                return Err("MCP response exceeds the 1 MiB message limit".into());
            }
            let message = serde_json::from_slice(&body).map_err(|_| "invalid MCP JSON response")?;
            parse_message(message, id)?
                .ok_or_else(|| "MCP HTTP response did not contain a result".into())
        }
        "text/event-stream" => read_sse(response, id),
        _ => Err("unsupported MCP HTTP response content type".into()),
    }
}

fn read_sse(response: Response, id: &Value) -> Result<Value, String> {
    let mut reader = BufReader::new(response.take((MAX_BYTES + 1) as u64));
    let mut total = 0;
    let mut data = String::new();
    let mut event = String::new();
    loop {
        let mut line = String::new();
        let count = reader
            .read_line(&mut line)
            .map_err(|_| "could not read MCP event stream; action outcome may be unknown")?;
        total += count;
        if total > MAX_BYTES {
            return Err("MCP event stream exceeds the 1 MiB message limit".into());
        }
        if count == 0 {
            return Err(
                "MCP event stream ended without a result; action outcome may be unknown".into(),
            );
        }
        let line = line.trim_end_matches(['\r', '\n']);
        if line.is_empty() {
            if !data.is_empty() && (event.is_empty() || event == "message") {
                let message = serde_json::from_str(&data).map_err(|_| "invalid MCP event JSON")?;
                if let Some(result) = parse_message(message, id)? {
                    return Ok(result);
                }
            }
            data.clear();
            event.clear();
        } else if let Some(value) = line.strip_prefix("data:") {
            if !data.is_empty() {
                data.push('\n');
            }
            data.push_str(value.strip_prefix(' ').unwrap_or(value));
        } else if let Some(value) = line.strip_prefix("event:") {
            event = value.strip_prefix(' ').unwrap_or(value).into();
        }
    }
}

fn parse_message(message: Value, id: &Value) -> Result<Option<Value>, String> {
    if !message.is_object() || message["jsonrpc"] != "2.0" {
        return Err("invalid MCP response envelope".into());
    }
    if let Some(method) = message.get("method") {
        if !method.is_string() || message.get("result").is_some() || message.get("error").is_some()
        {
            return Err("invalid MCP request envelope".into());
        }
        // This transport grants no sampling, elicitation, or filesystem authority.
        if message.get("id").is_some() {
            return Err("MCP HTTP server requested an unsupported client method".into());
        }
        return Ok(None);
    }
    if message.get("id") != Some(id)
        || message.get("result").is_some() == message.get("error").is_some()
    {
        return Err("invalid MCP response identity or result".into());
    }
    if message.get("error").is_some() {
        return Err("MCP server returned a protocol error".into());
    }
    message
        .get("result")
        .filter(|v| v.is_object())
        .cloned()
        .map(Some)
        .ok_or_else(|| "invalid MCP result".into())
}
