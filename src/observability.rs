//! Validated session and diagnostics policy; defaults live in configuration.
use serde::Deserialize;
use std::path::{Component, Path};
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ObservabilityConfig {
    pub session: SessionConfig,
    pub diagnostics: DiagnosticsConfig,
}
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SessionConfig {
    pub directory: String,
    pub timezone: String,
    pub sync_each_event: bool,
    pub max_index_bytes: usize,
    pub max_event_bytes: usize,
    pub max_session_bytes: usize,
    pub max_text_chars: usize,
}
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DiagnosticsConfig {
    pub level: String,
    pub max_file_bytes: u64,
    pub max_field_chars: usize,
    pub backup_count: usize,
    pub lock_timeout_ms: u64,
    pub lock_poll_ms: u64,
}
impl Default for ObservabilityConfig {
    fn default() -> Self {
        toml::from_str(include_str!("../config/observability.toml"))
            .expect("bundled observability policy")
    }
}
impl ObservabilityConfig {
    pub fn load() -> Result<Self, String> {
        let config = match std::env::var_os("ATLAS_OBSERVABILITY_CONFIG") {
            Some(path) => toml::from_str(
                &std::fs::read_to_string(path)
                    .map_err(|_| "Could not read observability configuration")?,
            )
            .map_err(|_| "Invalid observability configuration")?,
            None => Self::default(),
        };
        config.validate()?;
        Ok(config)
    }
    pub fn validate(&self) -> Result<(), String> {
        self.session.validate()?;
        let d = &self.diagnostics;
        if !["DEBUG", "INFO", "WARN", "ERROR"].contains(&d.level.as_str())
            || d.max_file_bytes == 0
            || d.max_field_chars == 0
            || d.backup_count == 0
            || d.backup_count > 100
            || d.lock_timeout_ms == 0
            || d.lock_poll_ms == 0
            || d.lock_poll_ms > d.lock_timeout_ms
        {
            return Err("Invalid diagnostics policy".into());
        }
        Ok(())
    }
}
impl SessionConfig {
    pub fn validate(&self) -> Result<(), String> {
        let path = Path::new(&self.directory);
        if self.directory.is_empty()
            || path.is_absolute()
            || path
                .components()
                .any(|c| !matches!(c, Component::Normal(_)))
            || !["local", "utc"].contains(&self.timezone.as_str())
            || self.max_index_bytes < 1024
            || self.max_session_bytes < self.max_event_bytes
            || self.max_event_bytes < 1024
            || self.max_text_chars == 0
            || self.max_text_chars > self.max_event_bytes / 4
        {
            return Err("Invalid session storage policy".into());
        }
        Ok(())
    }
    pub fn at(&self, time: chrono::DateTime<chrono::Utc>) -> chrono::DateTime<chrono::FixedOffset> {
        if self.timezone == "utc" {
            time.fixed_offset()
        } else {
            time.with_timezone(&chrono::Local).fixed_offset()
        }
    }
    pub fn now(&self) -> chrono::DateTime<chrono::FixedOffset> {
        self.at(chrono::Utc::now())
    }
}
