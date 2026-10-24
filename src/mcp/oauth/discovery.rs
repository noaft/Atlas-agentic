use reqwest::blocking::{Client, Response};
use reqwest::{header::WWW_AUTHENTICATE, Url};
use serde_json::{json, Value};
use std::io::Read;

const MAX_METADATA_BYTES: u64 = 1024 * 1024;

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::thread;

    fn fixture(
        responses: Vec<(&'static str, u16, Value, Option<String>)>,
    ) -> (String, thread::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let fixture_base = base.clone();
        let worker = thread::spawn(move || {
            for (path, status, body, challenge) in responses {
                let (mut stream, _) = listener.accept().unwrap();
                stream
                    .set_read_timeout(Some(std::time::Duration::from_secs(3)))
                    .unwrap();
                let mut bytes = Vec::new();
                let mut buf = [0; 1024];
                loop {
                    let count = stream.read(&mut buf).unwrap();
                    bytes.extend_from_slice(&buf[..count]);
                    if bytes.windows(4).any(|w| w == b"\r\n\r\n") {
                        break;
                    }
                }
                let request = String::from_utf8_lossy(&bytes);
                assert_eq!(
                    request
                        .lines()
                        .next()
                        .unwrap()
                        .split_whitespace()
                        .nth(1)
                        .unwrap(),
                    path
                );
                if path == "/register" {
                    assert!(request.starts_with("POST "));
                    let length: usize = request
                        .lines()
                        .find_map(|line| {
                            let (name, value) = line.split_once(':')?;
                            name.eq_ignore_ascii_case("content-length")
                                .then(|| value.trim().parse().unwrap())
                        })
                        .unwrap();
                    let header_end = bytes.windows(4).position(|w| w == b"\r\n\r\n").unwrap() + 4;
                    while bytes.len() < header_end + length {
                        let count = stream.read(&mut buf).unwrap();
                        assert!(count > 0);
                        bytes.extend_from_slice(&buf[..count]);
                    }
                    let registration: Value =
                        serde_json::from_slice(&bytes[header_end..header_end + length]).unwrap();
                    assert_eq!(
                        registration,
                        json!({
                            "client_name":"Atlas", "redirect_uris":["http://127.0.0.1:8765/callback"],
                            "grant_types":["authorization_code", "refresh_token"], "response_types":["code"],
                            "token_endpoint_auth_method":"none"
                        })
                    );
                }
                let body = body.to_string().replace("BASE", &fixture_base);
                let challenge = challenge
                    .map(|v| format!("WWW-Authenticate: {}\r\n", v.replace("BASE", &fixture_base)))
                    .unwrap_or_default();
                write!(stream, "HTTP/1.1 {status} OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n{challenge}Connection: close\r\n\r\n{body}", body.len()).unwrap();
            }
        });
        (base, worker)
    }

    fn client() -> Client {
        Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(std::time::Duration::from_secs(3))
            .build()
            .unwrap()
    }

    fn issuer_metadata() -> Value {
        json!({"issuer":"BASE/tenant", "authorization_endpoint":"BASE/authorize", "token_endpoint":"BASE/token", "registration_endpoint":"BASE/register", "code_challenge_methods_supported":["S256"], "token_endpoint_auth_methods_supported":["none"]})
    }

    #[test]
    fn endpoint_rejects_unsafe_urls() {
        for url in [
            "http://example.com/auth",
            "https://user:pass@example.com/auth",
            "https://example.com/auth#token",
            "file:///tmp/auth",
            "https://example.com/#",
        ] {
            assert!(endpoint(url).is_err(), "{url}");
        }
        assert!(endpoint("https://example.com/auth").is_ok());
    }

    #[test]
    fn rejects_oversized_metadata_before_reading_body() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let worker = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0; 4096];
            assert!(stream.read(&mut request).unwrap() > 0);
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                MAX_METADATA_BYTES + 1
            )
            .unwrap();
        });
        let response = client().get(url).send().unwrap();
        assert!(bounded_body(response).unwrap_err().contains("size limit"));
        worker.join().unwrap();
    }

    #[test]
    fn discovers_metadata_while_initial_sse_body_stays_open() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let server_base = base.clone();
        let (stop, stopped) = std::sync::mpsc::channel();
        let worker = thread::spawn(move || {
            let (mut sse, _) = listener.accept().unwrap();
            let mut request = [0; 4096];
            assert!(sse.read(&mut request).unwrap() > 0);
            write!(sse, "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nTransfer-Encoding: chunked\r\n\r\n").unwrap();
            listener.set_nonblocking(true).unwrap();
            for (path, body) in [
                (
                    "/.well-known/oauth-protected-resource/mcp",
                    json!({"resource":format!("{server_base}/mcp"), "authorization_servers":[format!("{server_base}/tenant")]}),
                ),
                (
                    "/.well-known/oauth-authorization-server/tenant",
                    issuer_metadata(),
                ),
            ] {
                let mut stream = loop {
                    match listener.accept() {
                        Ok((stream, _)) => break stream,
                        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                            if stopped.try_recv().is_ok() {
                                return;
                            }
                            thread::sleep(std::time::Duration::from_millis(5));
                        }
                        Err(error) => panic!("{error}"),
                    }
                };
                let count = stream.read(&mut request).unwrap();
                assert_eq!(
                    String::from_utf8_lossy(&request[..count])
                        .split_whitespace()
                        .nth(1),
                    Some(path)
                );
                let body = body.to_string().replace("BASE", &server_base);
                write!(
                    stream,
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                )
                .unwrap();
            }
            // Keep the first response open through both discovery requests.
            stopped.recv().unwrap();
            drop(sse);
        });
        let client = Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(std::time::Duration::from_secs(2))
            .build()
            .unwrap();
        let result = discover(&Url::parse(&format!("{base}/mcp")).unwrap(), &client);
        stop.send(()).unwrap();
        worker.join().unwrap();
        assert!(
            result.is_ok(),
            "discovery waited for the open SSE body: {result:?}"
        );
    }

    #[test]
    fn parses_bearer_parameters_and_quoted_commas() {
        let (metadata, scope) = bearer_challenge("Basic realm=\"elsewhere\", Bearer realm=\"a,b\", resource_metadata=\"https://example.com/meta?a=1,b=2\", scope=\"read write\"").unwrap();
        assert_eq!(
            metadata.as_deref(),
            Some("https://example.com/meta?a=1,b=2")
        );
        assert_eq!(scope.as_deref(), Some("read write"));
    }

    #[test]
    fn discovers_challenged_metadata_with_authoritative_scopes() {
        challenged_metadata_with_authoritative_scopes(401);
    }

    #[test]
    fn discovers_insufficient_scope_challenge_on_manual_reconnect() {
        challenged_metadata_with_authoritative_scopes(403);
    }

    fn challenged_metadata_with_authoritative_scopes(status: u16) {
        let (base, worker) = fixture(vec![
            (
                "/mcp",
                status,
                json!({}),
                Some("Bearer error=\"insufficient_scope\", resource_metadata=\"BASE/meta\", scope=\"read write\"".into()),
            ),
            (
                "/meta",
                200,
                json!({"resource":"BASE/mcp", "authorization_servers":["BASE/tenant"],"scopes_supported":["excessive"]}),
                None,
            ),
            (
                "/.well-known/oauth-authorization-server/tenant",
                200,
                issuer_metadata(),
                None,
            ),
        ]);
        let metadata = discover(&Url::parse(&format!("{base}/mcp")).unwrap(), &client()).unwrap();
        assert_eq!(metadata.scopes, ["read", "write"]);
        assert_eq!(metadata.issuer, format!("{base}/tenant"));
        worker.join().unwrap();
    }

    #[test]
    fn falls_back_to_resource_root_and_oidc_appended_path() {
        let (base, worker) = fixture(vec![
            ("/mcp", 401, json!({}), None),
            (
                "/.well-known/oauth-protected-resource/mcp",
                404,
                json!({}),
                None,
            ),
            (
                "/.well-known/oauth-protected-resource",
                200,
                json!({"resource":"BASE/mcp","authorization_servers":["BASE/tenant"],"scopes_supported":["read"]}),
                None,
            ),
            (
                "/.well-known/oauth-authorization-server/tenant",
                404,
                json!({}),
                None,
            ),
            (
                "/.well-known/openid-configuration/tenant",
                404,
                json!({}),
                None,
            ),
            (
                "/tenant/.well-known/openid-configuration",
                200,
                issuer_metadata(),
                None,
            ),
        ]);
        assert_eq!(
            discover(&Url::parse(&format!("{base}/mcp")).unwrap(), &client())
                .unwrap()
                .scopes,
            ["read"]
        );
        worker.join().unwrap();
    }

    #[test]
    fn rejects_resource_mismatch() {
        let (base, worker) = fixture(vec![
            ("/mcp", 401, json!({}), None),
            (
                "/.well-known/oauth-protected-resource/mcp",
                200,
                json!({"resource":"BASE/other","authorization_servers":["BASE/tenant"]}),
                None,
            ),
        ]);
        assert!(
            discover(&Url::parse(&format!("{base}/mcp")).unwrap(), &client())
                .unwrap_err()
                .contains("resource")
        );
        worker.join().unwrap();
    }

    #[test]
    fn rejects_issuer_mismatch_and_unsupported_pkce_or_public_clients() {
        for field in [
            "issuer",
            "code_challenge_methods_supported",
            "token_endpoint_auth_methods_supported",
        ] {
            let mut metadata = issuer_metadata();
            metadata[field] = match field {
                "issuer" => json!("BASE/other"),
                _ => json!([]),
            };
            let (base, worker) = fixture(vec![
                ("/mcp", 401, json!({}), None),
                (
                    "/.well-known/oauth-protected-resource/mcp",
                    200,
                    json!({"resource":"BASE/mcp","authorization_servers":["BASE/tenant"]}),
                    None,
                ),
                (
                    "/.well-known/oauth-authorization-server/tenant",
                    200,
                    metadata,
                    None,
                ),
            ]);
            assert!(discover(&Url::parse(&format!("{base}/mcp")).unwrap(), &client()).is_err());
            worker.join().unwrap();
        }
    }

    #[test]
    fn registers_public_client_and_rejects_secrets_or_changed_redirects() {
        for (response, success) in [
            (
                json!({"client_id":"atlas", "token_endpoint_auth_method":"none", "redirect_uris":["http://127.0.0.1:8765/callback"]}),
                true,
            ),
            (
                json!({"client_id":"atlas", "client_secret":"private"}),
                false,
            ),
            (
                json!({"client_id":"atlas", "redirect_uris":["https://other.example/callback"]}),
                false,
            ),
            (json!({"client_id":" "}), false),
        ] {
            let (base, worker) = fixture(vec![("/register", 201, response, None)]);
            let metadata = AuthorizationMetadata {
                resource: format!("{base}/mcp"),
                issuer: base.clone(),
                authorization_endpoint: Url::parse(&format!("{base}/authorize")).unwrap(),
                token_endpoint: Url::parse(&format!("{base}/token")).unwrap(),
                registration_endpoint: Some(Url::parse(&format!("{base}/register")).unwrap()),
                scopes: vec![],
            };
            assert_eq!(
                register(&client(), &metadata, "http://127.0.0.1:8765/callback").is_ok(),
                success
            );
            worker.join().unwrap();
        }
    }
}

#[derive(Debug)]
pub(super) struct AuthorizationMetadata {
    pub resource: String,
    pub issuer: String,
    pub authorization_endpoint: Url,
    pub token_endpoint: Url,
    pub registration_endpoint: Option<Url>,
    pub scopes: Vec<String>,
}

pub(super) fn endpoint(input: &str) -> Result<Url, String> {
    let url =
        Url::parse(input).map_err(|_| "OAuth metadata contains an invalid endpoint".to_owned())?;
    let test_loopback = cfg!(test)
        && url.scheme() == "http"
        && url.host_str().is_some_and(|host| {
            host.trim_matches(['[', ']'])
                .parse::<std::net::IpAddr>()
                .is_ok_and(|address| address.is_loopback())
        });
    if (url.scheme() != "https" && !test_loopback)
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.fragment().is_some()
    {
        return Err("OAuth endpoints require HTTPS without credentials or fragments".into());
    }
    Ok(url)
}

fn bounded_body(response: Response) -> Result<Vec<u8>, String> {
    if response
        .content_length()
        .is_some_and(|size| size > MAX_METADATA_BYTES)
    {
        return Err("OAuth discovery response exceeds the size limit".into());
    }
    let mut body = Vec::new();
    response
        .take(MAX_METADATA_BYTES + 1)
        .read_to_end(&mut body)
        .map_err(|_| "Could not read OAuth discovery response".to_owned())?;
    if body.len() as u64 > MAX_METADATA_BYTES {
        return Err("OAuth discovery response exceeds the size limit".into());
    }
    Ok(body)
}

fn metadata_at(client: &Client, urls: Vec<Url>) -> Result<Value, String> {
    for url in urls {
        let response = match client
            .get(url)
            .header(reqwest::header::ACCEPT, "application/json")
            .send()
        {
            Ok(response) => response,
            Err(_) => continue,
        };
        let success = response.status().is_success();
        let body = bounded_body(response)?;
        if success {
            return serde_json::from_slice(&body)
                .map_err(|_| "OAuth metadata is not valid JSON".into());
        }
    }
    Err("OAuth metadata could not be discovered".into())
}

fn identity(url: &Url) -> String {
    if url.path() == "/" && url.query().is_none() {
        url.as_str().trim_end_matches('/').to_owned()
    } else {
        url.as_str().to_owned()
    }
}

fn required_string<'a>(value: &'a Value, key: &str) -> Result<&'a str, String> {
    value
        .get(key)
        .and_then(Value::as_str)
        .filter(|v| !v.trim().is_empty())
        .ok_or_else(|| "OAuth metadata is missing a required string".into())
}

fn supported(value: &Value, key: &str, method: &str) -> bool {
    value
        .get(key)
        .and_then(Value::as_array)
        .is_some_and(|values| values.iter().any(|value| value.as_str() == Some(method)))
}

fn well_known(base: &Url, name: &str) -> Url {
    let mut url = base.clone();
    url.set_query(None);
    url.set_fragment(None);
    let suffix = base.path().trim_end_matches('/');
    url.set_path(&format!("/.well-known/{name}{suffix}"));
    url
}

pub(super) fn discover(resource: &Url, client: &Client) -> Result<AuthorizationMetadata, String> {
    endpoint(resource.as_str())?;
    let response = client
        .get(resource.clone())
        .header(
            reqwest::header::ACCEPT,
            "application/json, text/event-stream",
        )
        .send()
        .map_err(|_| "Could not request MCP authorization information".to_owned())?;
    let mut challenged_metadata = None;
    let mut challenged_scope = None;
    if matches!(
        response.status(),
        reqwest::StatusCode::UNAUTHORIZED | reqwest::StatusCode::FORBIDDEN
    ) {
        for header in response.headers().get_all(WWW_AUTHENTICATE) {
            let header = header
                .to_str()
                .map_err(|_| "Invalid MCP authorization challenge".to_owned())?;
            let (metadata, scope) = bearer_challenge(header)?;
            if challenged_metadata.is_none() {
                challenged_metadata = metadata;
            }
            if challenged_scope.is_none() {
                challenged_scope = scope;
            }
        }
    }
    // MCP GET can return a persistent SSE stream. Discovery needs only its headers.
    drop(response);
    let resource_urls = if let Some(metadata) = challenged_metadata {
        vec![endpoint(&metadata)?]
    } else {
        let specific = well_known(resource, "oauth-protected-resource");
        let mut root = resource.clone();
        root.set_path("/.well-known/oauth-protected-resource");
        root.set_query(None);
        if root == specific {
            vec![root]
        } else {
            vec![specific, root]
        }
    };
    let protected = metadata_at(client, resource_urls)?;
    let advertised_resource = endpoint(required_string(&protected, "resource")?)?;
    if identity(&advertised_resource) != identity(resource) {
        return Err("OAuth protected resource does not match the MCP server".into());
    }
    let issuer = protected
        .get("authorization_servers")
        .and_then(Value::as_array)
        .and_then(|servers| servers.first())
        .and_then(Value::as_str)
        .ok_or_else(|| "OAuth resource metadata has no authorization server".to_owned())?;
    let issuer = endpoint(issuer)?;
    if issuer.query().is_some() {
        return Err("OAuth issuer must not contain a query".into());
    }
    let mut issuer_urls = vec![
        well_known(&issuer, "oauth-authorization-server"),
        well_known(&issuer, "openid-configuration"),
    ];
    if issuer.path() != "/" {
        let mut appended = issuer.clone();
        appended.set_path(&format!(
            "{}/.well-known/openid-configuration",
            issuer.path().trim_end_matches('/')
        ));
        issuer_urls.push(appended);
    }
    let metadata = metadata_at(client, issuer_urls)?;
    if identity(&endpoint(required_string(&metadata, "issuer")?)?) != identity(&issuer) {
        return Err(
            "OAuth metadata issuer does not match the selected authorization server".into(),
        );
    }
    if !supported(&metadata, "code_challenge_methods_supported", "S256") {
        return Err("OAuth server must advertise PKCE S256 support".into());
    }
    if !supported(&metadata, "token_endpoint_auth_methods_supported", "none") {
        return Err("OAuth server must advertise public client authentication (none)".into());
    }
    let scopes = match challenged_scope {
        Some(scope) => scope.split_whitespace().map(str::to_owned).collect(),
        None => protected
            .get("scopes_supported")
            .and_then(Value::as_array)
            .map(|scopes| {
                scopes
                    .iter()
                    .map(|scope| {
                        scope
                            .as_str()
                            .filter(|s| !s.is_empty() && !s.chars().any(char::is_whitespace))
                            .map(str::to_owned)
                            .ok_or_else(|| "OAuth metadata has invalid scopes".to_owned())
                    })
                    .collect::<Result<Vec<_>, _>>()
            })
            .transpose()?
            .unwrap_or_default(),
    };
    Ok(AuthorizationMetadata {
        resource: resource.as_str().to_owned(),
        issuer: issuer.as_str().to_owned(),
        authorization_endpoint: endpoint(required_string(&metadata, "authorization_endpoint")?)?,
        token_endpoint: endpoint(required_string(&metadata, "token_endpoint")?)?,
        registration_endpoint: metadata
            .get("registration_endpoint")
            .map(|value| {
                value
                    .as_str()
                    .ok_or_else(|| "OAuth metadata has an invalid registration endpoint".to_owned())
                    .and_then(endpoint)
            })
            .transpose()?,
        scopes,
    })
}

pub(super) fn register(
    client: &Client,
    metadata: &AuthorizationMetadata,
    redirect_uri: &str,
) -> Result<String, String> {
    let url = metadata.registration_endpoint.as_ref().ok_or_else(|| {
        "OAuth server does not support registration; configure a pre-registered client ID"
            .to_owned()
    })?;
    let response = client
        .post(url.clone())
        .json(&json!({
            "client_name":"Atlas", "redirect_uris":[redirect_uri],
            "grant_types":["authorization_code","refresh_token"], "response_types":["code"],
            "token_endpoint_auth_method":"none"
        }))
        .send()
        .map_err(|_| "OAuth client registration request failed".to_owned())?;
    let success = response.status().is_success();
    let body = bounded_body(response)?;
    if !success {
        return Err("OAuth server rejected client registration".into());
    }
    let registration: Value = serde_json::from_slice(&body)
        .map_err(|_| "Invalid OAuth registration response".to_owned())?;
    if registration.get("client_secret").is_some()
        || registration
            .get("token_endpoint_auth_method")
            .is_some_and(|v| v.as_str() != Some("none"))
        || registration.get("redirect_uris").is_some_and(|value| {
            value
                .as_array()
                .is_none_or(|uris| uris.len() != 1 || uris[0].as_str() != Some(redirect_uri))
        })
    {
        return Err("OAuth registration did not preserve public client settings".into());
    }
    Ok(required_string(&registration, "client_id")?.to_owned())
}

// Split challenges only at unquoted commas, retaining escaped quoted values.
fn bearer_challenge(header: &str) -> Result<(Option<String>, Option<String>), String> {
    let mut parts = Vec::new();
    let mut start = 0;
    let mut quoted = false;
    let mut escaped = false;
    for (index, ch) in header.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        if quoted && ch == '\\' {
            escaped = true;
        } else if ch == '"' {
            quoted = !quoted;
        } else if ch == ',' && !quoted {
            parts.push(&header[start..index]);
            start = index + 1;
        }
    }
    if quoted || escaped {
        return Err("Invalid MCP authorization challenge".into());
    }
    parts.push(&header[start..]);
    let mut bearer = false;
    let mut metadata = None;
    let mut scope = None;
    for part in parts {
        let mut part = part.trim();
        let token_end = part
            .find(|c: char| c.is_whitespace() || c == '=')
            .unwrap_or(part.len());
        let rest = part[token_end..].trim_start();
        if !rest.starts_with('=') {
            bearer = part[..token_end].eq_ignore_ascii_case("Bearer");
            part = rest;
        }
        if !bearer || part.is_empty() {
            continue;
        }
        let Some((key, value)) = part.split_once('=') else {
            continue;
        };
        let value = value.trim();
        let value = if value.starts_with('"') {
            if !value.ends_with('"') || value.len() < 2 {
                return Err("Invalid MCP authorization challenge".into());
            }
            let mut decoded = String::new();
            let mut chars = value[1..value.len() - 1].chars();
            while let Some(ch) = chars.next() {
                if ch == '\\' {
                    decoded.push(chars.next().ok_or("Invalid MCP authorization challenge")?);
                } else {
                    decoded.push(ch);
                }
            }
            decoded
        } else {
            value.to_owned()
        };
        if key.trim().eq_ignore_ascii_case("resource_metadata") {
            metadata = Some(value);
        } else if key.trim().eq_ignore_ascii_case("scope") {
            scope = Some(value);
        }
    }
    Ok((metadata, scope))
}
