use std::{collections::BTreeMap, fs, path::Path};

use serde::Deserialize;

/// User-local, explicitly trusted MCP configuration. Never load this from a workspace.
#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct McpConfig {
    #[serde(default)]
    pub servers: BTreeMap<String, ServerConfig>,
}

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ServerConfig {
    #[serde(default)]
    pub command: String,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub oauth: Option<OAuthConfig>,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default = "default_timeout")]
    pub timeout_ms: u64,
}

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OAuthConfig {
    #[serde(default)]
    pub client_id: Option<String>,
    #[serde(default)]
    pub scopes: Vec<String>,
    #[serde(default)]
    pub redirect_port: Option<u16>,
}

/// Remote endpoints use TLS except for literal loopback addresses used by local servers.
pub(crate) fn validate_http_url(value: &str) -> Result<reqwest::Url, String> {
    let url = reqwest::Url::parse(value).map_err(|_| "invalid MCP HTTP URL")?;
    let loopback = url.host_str().is_some_and(|host| {
        host.trim_matches(['[', ']'])
            .parse::<std::net::IpAddr>()
            .is_ok_and(|address| address.is_loopback())
    });
    if !matches!(url.scheme(), "https" | "http")
        || (url.scheme() == "http" && !loopback)
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.fragment().is_some()
        || url.query().is_some()
    {
        return Err("MCP URL requires HTTPS (or literal loopback HTTP), without userinfo, query, or fragment".into());
    }
    Ok(url)
}

fn default_timeout() -> u64 {
    30_000
}

impl McpConfig {
    pub fn from_toml(input: &str) -> Result<Self, String> {
        // TOML errors can quote source lines containing credentials or command arguments.
        let config: Self = toml::from_str(input).map_err(|_| "invalid MCP configuration")?;
        for (name, server) in &config.servers {
            if name.is_empty()
                || name.len() > 64
                || !name
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'_' | b'-'))
            {
                return Err(
                    "MCP server names must use 1–64 ASCII letters, digits, '_' or '-'".into(),
                );
            }
            let has_command = !server.command.trim().is_empty();
            if has_command == server.url.is_some() {
                return Err("MCP server requires exactly one command or URL".into());
            }
            if let Some(url) = &server.url {
                validate_http_url(url)?;
                if !server.args.is_empty() {
                    return Err("MCP HTTP servers cannot have command arguments".into());
                }
            }
            if let Some(oauth) = &server.oauth {
                if server.url.is_none()
                    || oauth.redirect_port == Some(0)
                    || oauth.client_id.as_ref().is_some_and(|value| {
                        value.trim().is_empty()
                            || value.len() > 4096
                            || value.chars().any(char::is_control)
                    })
                    || oauth.scopes.len() > 64
                    || oauth.scopes.iter().any(|value| {
                        value.trim().is_empty()
                            || value.len() > 1024
                            || value.chars().any(char::is_whitespace)
                            || value.chars().any(char::is_control)
                    })
                {
                    return Err("invalid MCP OAuth configuration".into());
                }
            }
            if server.command.contains('\0') || server.args.iter().any(|arg| arg.contains('\0')) {
                return Err(
                    "MCP command must be nonblank and command arguments must not contain NUL"
                        .into(),
                );
            }
            if !(1..=300_000).contains(&server.timeout_ms) {
                return Err("MCP timeout_ms must be between 1 and 300000".into());
            }
        }
        Ok(config)
    }

    pub fn load(path: &Path) -> Result<Self, String> {
        match fs::read_to_string(path) {
            Ok(input) => Self::from_toml(&input),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(_) => Err("could not read MCP configuration".into()),
        }
    }

    pub fn from_environment() -> Result<Self, String> {
        let path = if let Some(path) = std::env::var_os("ATLAS_MCP_CONFIG") {
            std::path::PathBuf::from(path)
        } else {
            let directory = match std::env::var_os("ATLAS_CONFIG_DIR") {
                Some(path) => std::path::PathBuf::from(path),
                None => directories::BaseDirs::new()
                    .ok_or("could not resolve the user configuration directory")?
                    .config_dir()
                    .join("atlas"),
            };
            directory.join("mcp.toml")
        };
        Self::load(&path)
    }
}
