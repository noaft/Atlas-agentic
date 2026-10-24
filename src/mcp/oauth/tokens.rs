use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fmt,
    fs::{File, OpenOptions},
    io::{self, Read, Seek, SeekFrom, Write},
    path::PathBuf,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

pub(super) fn identity(resource: &str, client: &Option<String>) -> String {
    let encoded = serde_json::to_vec(&(resource, client)).unwrap_or_default();
    format!("{:x}", Sha256::digest(encoded))
}

pub(super) fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

/// The private lock file carries only a nonsecret invalidation counter. Call under its lease.
pub(super) fn generation(lease: &Option<File>) -> Result<u64, String> {
    let Some(file) = lease else {
        return Ok(0);
    };
    let mut file = file
        .try_clone()
        .map_err(|_| "could not read MCP credential generation")?;
    file.seek(SeekFrom::Start(0))
        .map_err(|_| "could not read MCP credential generation")?;
    let mut value = String::new();
    file.take(32)
        .read_to_string(&mut value)
        .map_err(|_| "could not read MCP credential generation")?;
    if value.is_empty() {
        Ok(0)
    } else {
        value
            .trim()
            .parse()
            .map_err(|_| "invalid MCP credential generation".into())
    }
}

pub(super) fn invalidate(lease: &Option<File>) -> Result<(), String> {
    let next = generation(lease)?
        .checked_add(1)
        .ok_or("MCP credential generation exhausted")?;
    let Some(file) = lease else {
        return Ok(());
    };
    let mut file = file
        .try_clone()
        .map_err(|_| "could not invalidate MCP credentials")?;
    file.seek(SeekFrom::Start(0))
        .and_then(|_| file.set_len(0))
        .and_then(|_| write!(file, "{next}"))
        .and_then(|_| file.sync_all())
        .map_err(|_| "could not invalidate MCP credentials".into())
}

#[derive(Clone, Serialize, Deserialize)]
pub(super) struct Token {
    pub access_token: String,
    pub refresh_token: Option<String>,
    pub expires_at: Option<u64>,
    pub resource: String,
    pub client_id: String,
    pub token_endpoint: String,
}

impl fmt::Debug for Token {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("OAuthToken([redacted])")
    }
}

impl Token {
    pub fn from_response(
        response: serde_json::Value,
        resource: &str,
        client: &str,
        endpoint: &str,
    ) -> Result<Self, String> {
        if !response["token_type"]
            .as_str()
            .is_some_and(|kind| kind.eq_ignore_ascii_case("Bearer"))
        {
            return Err("OAuth server did not return a Bearer token".into());
        }
        let access_token = response["access_token"]
            .as_str()
            .filter(|token| valid_token(token))
            .ok_or("OAuth server returned an invalid access token")?
            .to_string();
        let refresh_token = match response.get("refresh_token") {
            Some(value) => Some(
                value
                    .as_str()
                    .filter(|token| valid_token(token))
                    .ok_or("OAuth server returned an invalid refresh token")?
                    .to_string(),
            ),
            None => None,
        };
        let expires_at = match response.get("expires_in") {
            Some(value) => Some(
                now().saturating_add(
                    value
                        .as_u64()
                        .filter(|value| *value > 0)
                        .ok_or("OAuth server returned an invalid token lifetime")?,
                ),
            ),
            None => None,
        };
        crate::diagnostics::protect_secret(&access_token);
        if let Some(token) = &refresh_token {
            crate::diagnostics::protect_secret(token);
        }
        Ok(Self {
            access_token,
            refresh_token,
            expires_at,
            resource: resource.into(),
            client_id: client.into(),
            token_endpoint: endpoint.into(),
        })
    }

    pub fn expired(&self) -> bool {
        self.expires_at
            .is_some_and(|expiry| expiry <= now().saturating_add(30))
    }
}

fn valid_token(token: &str) -> bool {
    !token.is_empty()
        && token.len() <= 64 * 1024
        && !token.chars().any(char::is_control)
        && !token.chars().any(char::is_whitespace)
}

pub(super) trait TokenStore: Send + Sync {
    fn load(&self, key: &str) -> Result<Option<Token>, String>;
    fn save(&self, key: &str, token: &Token) -> Result<(), String>;
    fn delete(&self, key: &str) -> Result<(), String>;
    fn lease(&self, _key: &str) -> Result<Option<File>, String> {
        Ok(None)
    }
}

pub(super) struct KeyringStore;

impl KeyringStore {
    fn entry(key: &str) -> Result<keyring::Entry, String> {
        keyring::Entry::new("atlas-mcp-oauth", key).map_err(|_| {
            "OS credential store unavailable; unlock or enable your system keyring".into()
        })
    }
}

impl TokenStore for KeyringStore {
    fn load(&self, key: &str) -> Result<Option<Token>, String> {
        match Self::entry(key)?.get_password() {
            Ok(value) => {
                let token: Token = serde_json::from_str(&value)
                    .map_err(|_| "invalid saved MCP credentials; disconnect and sign in again")?;
                if !valid_token(&token.access_token)
                    || token
                        .refresh_token
                        .as_ref()
                        .is_some_and(|token| !valid_token(token))
                {
                    return Err(
                        "invalid saved MCP credentials; disconnect and sign in again".into(),
                    );
                }
                crate::diagnostics::protect_secret(&token.access_token);
                if let Some(refresh) = &token.refresh_token {
                    crate::diagnostics::protect_secret(refresh);
                }
                Ok(Some(token))
            }
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(_) => {
                Err("could not read MCP credentials; unlock or enable your system keyring".into())
            }
        }
    }

    fn save(&self, key: &str, token: &Token) -> Result<(), String> {
        let value = serde_json::to_string(token).map_err(|_| "could not encode MCP credentials")?;
        Self::entry(key)?.set_password(&value).map_err(|_| "could not save MCP credentials in the OS credential store; no plaintext fallback is used".into())
    }

    fn delete(&self, key: &str) -> Result<(), String> {
        match Self::entry(key)?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(_) => Err("could not remove MCP credentials from the OS credential store".into()),
        }
    }

    fn lease(&self, key: &str) -> Result<Option<File>, String> {
        let directory = match std::env::var_os("ATLAS_CONFIG_DIR") {
            Some(path) => PathBuf::from(path),
            None => directories::BaseDirs::new()
                .ok_or("could not resolve the configuration directory")?
                .config_dir()
                .join("atlas"),
        }
        .join("mcp-auth-locks");
        let mut builder = std::fs::DirBuilder::new();
        builder.recursive(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            builder.mode(0o700);
        }
        builder
            .create(&directory)
            .map_err(|_| "could not create MCP credential lock directory")?;
        let mut options = OpenOptions::new();
        options.create(true).read(true).write(true).truncate(false);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let file = options
            .open(directory.join(format!("{key}.lock")))
            .map_err(|_| "could not open MCP credential lock")?;
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            match fs2::FileExt::try_lock_exclusive(&file) {
                Ok(()) => return Ok(Some(file)),
                Err(error)
                    if error.kind() == io::ErrorKind::WouldBlock && Instant::now() < deadline =>
                {
                    std::thread::sleep(Duration::from_millis(20))
                }
                Err(_) => return Err("MCP credentials are busy; retry shortly".into()),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn credential_identity_is_bound_to_resource_and_client_configuration() {
        assert_ne!(
            identity("https://one.example/mcp", &Some("client".into())),
            identity("https://two.example/mcp", &Some("client".into()))
        );
        assert_ne!(
            identity("https://one.example/mcp", &Some("client".into())),
            identity("https://one.example/mcp", &Some("other".into()))
        );
    }

    #[test]
    fn tokens_validate_bearer_type_and_never_debug_credentials() {
        let input = serde_json::json!({"access_token":"private-access","refresh_token":"private-refresh","token_type":"Bearer","expires_in":60});
        let token = Token::from_response(
            input,
            "https://server.example/mcp",
            "client",
            "https://auth.example/token",
        )
        .unwrap();
        assert!(!format!("{token:?}").contains("private"));
        assert!(!token.expired());
        assert!(Token::from_response(
            serde_json::json!({"access_token":"secret","token_type":"MAC"}),
            "resource",
            "client",
            "endpoint"
        )
        .is_err());
    }
}
