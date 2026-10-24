use std::{collections::BTreeMap, fs, path::Path};

use serde::Deserialize;

/// Local, explicitly trusted process configuration. Never load this from a workspace.
#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct McpConfig {
    #[serde(default)]
    pub servers: BTreeMap<String, ServerConfig>,
}

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ServerConfig {
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default = "default_timeout")]
    pub timeout_ms: u64,
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
            if server.command.trim().is_empty()
                || server.command.contains('\0')
                || server.args.iter().any(|arg| arg.contains('\0'))
            {
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
