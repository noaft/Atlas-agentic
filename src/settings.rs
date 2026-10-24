use std::{
    collections::BTreeSet,
    env,
    ffi::OsString,
    fmt, fs,
    fs::{File, OpenOptions},
    io,
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::Arc,
};

use directories::BaseDirs;
use fs2::FileExt;
use serde::{Deserialize, Serialize};

use crate::permissions::PermissionMode;

#[cfg(unix)]
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};

mod migration;
pub mod providers;
pub use providers::{
    ModelSource, ProviderCapabilities, ProviderId, ProviderSettings, ScopedModelLimits,
};

pub const SETTINGS_VERSION: u32 = 2;
pub const DEFAULT_DEEPSEEK_BASE_URL: &str = "https://api.deepseek.com";
pub const DEFAULT_DEEPSEEK_MODEL: &str = "deepseek-flash";
pub const DEFAULT_HISTORY_LIMIT: usize = 40;

type SettingsPublisher = dyn Fn(&Path, &Path) -> io::Result<()> + Send + Sync;

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AtlasSettings {
    pub version: u32,
    #[serde(default)]
    pub selected_provider: Option<ProviderId>,
    #[serde(default)]
    pub providers: std::collections::BTreeMap<String, ProviderSettings>,
    pub deepseek: DeepSeekSettings,
    pub conversation: ConversationSettings,
    pub workspace: WorkspaceSettings,
    #[serde(default)]
    pub permissions: PermissionSettings,
}

impl fmt::Debug for AtlasSettings {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AtlasSettings")
            .field("version", &self.version)
            .field("deepseek", &self.deepseek)
            .field("conversation", &self.conversation)
            .field("workspace", &self.workspace)
            .field("permissions", &self.permissions)
            .finish()
    }
}

impl Default for AtlasSettings {
    fn default() -> Self {
        Self {
            version: SETTINGS_VERSION,
            selected_provider: None,
            providers: Default::default(),
            deepseek: DeepSeekSettings::default(),
            conversation: ConversationSettings::default(),
            workspace: WorkspaceSettings::default(),
            permissions: PermissionSettings::default(),
        }
    }
}

impl AtlasSettings {
    pub fn validate(&self) -> Result<(), String> {
        if self.version != SETTINGS_VERSION {
            return Err(format!("unsupported settings version: {}", self.version));
        }
        for (id, profile) in &self.providers {
            profile.validate(ProviderId::parse(id)?)?;
        }
        if self.deepseek.base_url.trim().is_empty() {
            return Err("DeepSeek base URL cannot be blank".to_string());
        }
        providers::normalize_url(&self.deepseek.base_url)?;
        if self.deepseek.model.trim().is_empty() {
            return Err("DeepSeek model cannot be blank".to_string());
        }
        for limits in self.conversation.model_limits.values() {
            limits.validate()?;
        }
        if self.conversation.history_limit == 0 {
            return Err("conversation history limit must be a positive integer".to_string());
        }
        Ok(())
    }

    pub fn resolve_with(
        &self,
        environment: impl Fn(&str) -> Option<OsString>,
    ) -> Result<EffectiveSettings, String> {
        self.resolve_for("deepseek", DEFAULT_DEEPSEEK_MODEL, environment)
    }

    pub fn provider_profile(&self, provider: ProviderId) -> ProviderSettings {
        let mut profile = self
            .providers
            .get(provider.as_str())
            .cloned()
            .unwrap_or_else(|| ProviderSettings::defaults(provider));
        if profile.base_url.is_empty() && provider != ProviderId::OpenaiCompatible {
            profile.base_url = provider.descriptor().base_url;
        }
        if provider == ProviderId::Deepseek {
            profile.api_key = self.deepseek.api_key.clone();
            profile.base_url = self.deepseek.base_url.clone();
            if profile.model.is_none() && !self.providers.contains_key("deepseek") {
                profile.model = Some(self.deepseek.model.clone());
            }
            if !self.providers.contains_key("deepseek") {
                if let Ok(source) = providers::source_for(provider, &profile.base_url) {
                    for (model, limits) in &self.conversation.model_limits {
                        profile
                            .model_limits
                            .entry(model.clone())
                            .or_insert_with(|| ScopedModelLimits {
                                source_id: source.source_id.clone(),
                                limits: *limits,
                            });
                    }
                }
            }
        }
        profile
    }

    pub fn resolve_for(
        &self,
        agent_provider: &str,
        agent_model: &str,
        environment: impl Fn(&str) -> Option<OsString>,
    ) -> Result<EffectiveSettings, String> {
        self.validate()?;
        let env_provider = environment_string(&environment, "ATLAS_PROVIDER")?;
        let provider = if let Some(value) = env_provider.as_deref() {
            ProviderId::parse(value)?
        } else {
            match self.selected_provider {
                Some(provider) => provider,
                None => ProviderId::parse(agent_provider)?,
            }
        };
        let profile = self.provider_profile(provider);
        let descriptor = provider.descriptor();
        let prefix = descriptor.env_prefix;
        let mut effective = EffectiveSettings {
            provider,
            source: None,
            capabilities: profile.capabilities.clone(),
            allow_keyless: profile.allow_keyless,
            model_from_agent: false,
            request_output_tokens: profile
                .request_output_tokens
                .unwrap_or(descriptor.request_output_tokens),
            api_key: profile.api_key.clone(),
            base_url: profile.base_url.clone(),
            model: profile.model.clone().unwrap_or_default(),
            history_limit: self.conversation.history_limit,
            model_limits: Default::default(),
            default_workspace: self.workspace.default_path.clone(),
            default_permission_mode: self.permissions.default_mode,
            environment_overrides: BTreeSet::new(),
        };
        if env_provider.is_some() {
            effective
                .environment_overrides
                .insert(SettingField::Provider);
        }
        if let Some(value) = environment_string(&environment, &format!("{prefix}_API_KEY"))? {
            effective.api_key = Some(value);
            effective.environment_overrides.insert(SettingField::ApiKey);
        }
        if let Some(value) = environment_string(&environment, &format!("{prefix}_BASE_URL"))? {
            effective.base_url = value;
            effective
                .environment_overrides
                .insert(SettingField::BaseUrl);
        }
        if let Some(value) = environment_string(&environment, &format!("{prefix}_MODEL"))? {
            effective.model = value;
            effective.environment_overrides.insert(SettingField::Model);
        } else if provider.as_str() == agent_provider
            && (effective.model.is_empty()
                || (provider == ProviderId::Deepseek
                    && effective.model == DEFAULT_DEEPSEEK_MODEL
                    && !self.providers.contains_key("deepseek")))
        {
            effective.model = agent_model.to_string();
            effective.model_from_agent = true;
        }
        if !effective.base_url.is_empty() {
            effective.source = Some(providers::source_for(provider, &effective.base_url)?);
            effective.model_limits = profile
                .model_limits
                .iter()
                .filter(|(_, limits)| {
                    Some(limits.source_id.as_str())
                        == effective.source.as_ref().map(|s| s.source_id.as_str())
                })
                .map(|(m, l)| (m.clone(), l.limits))
                .collect();
        }
        if let Some(value) = environment("ATLAS_HISTORY_MESSAGES") {
            let value = value.into_string().map_err(|_| history_limit_error())?;
            effective.history_limit = value
                .parse::<usize>()
                .ok()
                .filter(|limit| *limit > 0)
                .ok_or_else(history_limit_error)?;
            effective
                .environment_overrides
                .insert(SettingField::HistoryLimit);
        }

        effective.validate()?;
        Ok(effective)
    }

    pub fn resolve_from_env(&self) -> Result<EffectiveSettings, String> {
        self.resolve_with(|name| env::var_os(name))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum SettingField {
    Provider,
    ApiKey,
    BaseUrl,
    Model,
    HistoryLimit,
}

#[derive(Clone, PartialEq, Eq)]
pub struct EffectiveSettings {
    pub provider: ProviderId,
    pub source: Option<ModelSource>,
    pub capabilities: ProviderCapabilities,
    pub allow_keyless: bool,
    pub request_output_tokens: u64,
    pub model_from_agent: bool,
    pub api_key: Option<String>,
    pub base_url: String,
    pub model: String,
    pub history_limit: usize,
    pub model_limits: std::collections::BTreeMap<String, crate::models::ModelLimits>,
    pub default_workspace: Option<PathBuf>,
    pub default_permission_mode: PermissionMode,
    pub environment_overrides: BTreeSet<SettingField>,
}

impl Default for EffectiveSettings {
    fn default() -> Self {
        AtlasSettings::default()
            .resolve_with(|_| None)
            .expect("default Atlas settings must be valid")
    }
}

impl fmt::Debug for EffectiveSettings {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("EffectiveSettings")
            .field("api_key_configured", &self.api_key.is_some())
            .field("base_url", &self.base_url)
            .field("model", &self.model)
            .field("history_limit", &self.history_limit)
            .field("default_workspace", &self.default_workspace)
            .field("default_permission_mode", &self.default_permission_mode)
            .field("environment_overrides", &self.environment_overrides)
            .finish()
    }
}

impl EffectiveSettings {
    fn validate(&self) -> Result<(), String> {
        if !self.base_url.is_empty() {
            providers::normalize_url(&self.base_url)?;
        }
        if self.history_limit == 0 {
            return Err("conversation history limit must be a positive integer".to_string());
        }
        Ok(())
    }
}

fn environment_string(
    environment: &impl Fn(&str) -> Option<OsString>,
    name: &str,
) -> Result<Option<String>, String> {
    environment(name)
        .map(|value| {
            value
                .into_string()
                .map_err(|_| format!("{name} must contain valid Unicode"))
        })
        .transpose()
}

fn history_limit_error() -> String {
    "ATLAS_HISTORY_MESSAGES must be a positive integer".to_string()
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeepSeekSettings {
    pub api_key: Option<String>,
    pub base_url: String,
    pub model: String,
}

impl fmt::Debug for DeepSeekSettings {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("DeepSeekSettings")
            .field("api_key_configured", &self.api_key.is_some())
            .field("base_url", &self.base_url)
            .field("model", &self.model)
            .finish()
    }
}

impl Default for DeepSeekSettings {
    fn default() -> Self {
        Self {
            api_key: None,
            base_url: DEFAULT_DEEPSEEK_BASE_URL.to_string(),
            model: DEFAULT_DEEPSEEK_MODEL.to_string(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConversationSettings {
    pub history_limit: usize,
    #[serde(default)]
    pub model_limits: std::collections::BTreeMap<String, crate::models::ModelLimits>,
}

impl Default for ConversationSettings {
    fn default() -> Self {
        Self {
            history_limit: DEFAULT_HISTORY_LIMIT,
            model_limits: Default::default(),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceSettings {
    pub default_path: Option<PathBuf>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PermissionSettings {
    #[serde(default)]
    pub default_mode: PermissionMode,
}

#[derive(Clone)]
pub struct SettingsStore {
    path: PathBuf,
    publisher: Arc<SettingsPublisher>,
}

impl fmt::Debug for SettingsStore {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SettingsStore")
            .field("path", &self.path)
            .finish_non_exhaustive()
    }
}

impl SettingsStore {
    pub fn directory(&self) -> &std::path::Path {
        self.path
            .parent()
            .unwrap_or_else(|| std::path::Path::new("."))
    }

    pub fn from_user_config() -> Result<Self, String> {
        let directory = match env::var_os("ATLAS_CONFIG_DIR") {
            Some(path) => PathBuf::from(path),
            None => BaseDirs::new()
                .ok_or_else(|| "could not resolve the user configuration directory".to_string())?
                .config_dir()
                .join("atlas"),
        };
        Ok(Self::at(directory.join("settings.json")))
    }

    pub fn at(path: PathBuf) -> Self {
        Self::at_with_publisher(path, |source, destination| fs::rename(source, destination))
    }

    #[doc(hidden)]
    pub fn at_with_publisher(
        path: PathBuf,
        publisher: impl Fn(&Path, &Path) -> io::Result<()> + Send + Sync + 'static,
    ) -> Self {
        Self {
            path,
            publisher: Arc::new(publisher),
        }
    }

    pub fn load(&self) -> Result<AtlasSettings, String> {
        let lock = self.open_lock()?;
        FileExt::lock_shared(&lock)
            .map_err(|error| format!("failed to lock {}: {error}", self.lock_path().display()))?;
        self.read_unlocked()
    }

    pub fn save(&self, settings: &AtlasSettings) -> Result<(), String> {
        settings.validate()?;
        let lock = self.open_lock()?;
        lock.lock_exclusive()
            .map_err(|error| format!("failed to lock {}: {error}", self.lock_path().display()))?;
        self.write_unlocked(settings)
    }

    pub fn update(
        &self,
        mutation: impl FnOnce(&mut AtlasSettings) -> Result<(), String>,
    ) -> Result<AtlasSettings, String> {
        let lock = self.open_lock()?;
        lock.lock_exclusive()
            .map_err(|error| format!("failed to lock {}: {error}", self.lock_path().display()))?;
        let mut settings = self.read_unlocked()?;
        mutation(&mut settings)?;
        settings.validate()?;
        self.write_unlocked(&settings)?;
        Ok(settings)
    }

    fn read_unlocked(&self) -> Result<AtlasSettings, String> {
        if !self.path.exists() {
            return Ok(AtlasSettings::default());
        }
        let mut input = String::new();
        File::open(&self.path)
            .and_then(|mut file| file.read_to_string(&mut input))
            .map_err(|error| format!("failed to read {}: {error}", self.path.display()))?;
        let mut settings: AtlasSettings = serde_json::from_str(&input)
            .map_err(|error| format!("failed to parse {}: {error}", self.path.display()))?;
        migration::upgrade(&mut settings);
        settings.validate()?;
        Ok(settings)
    }

    fn write_unlocked(&self, settings: &AtlasSettings) -> Result<(), String> {
        let temporary = self.temporary_path();
        let bytes = serde_json::to_vec_pretty(settings)
            .map_err(|error| format!("failed to serialize settings: {error}"))?;
        let mut output = private_file_options()
            .create(true)
            .truncate(true)
            .write(true)
            .open(&temporary)
            .map_err(|error| format!("failed to create {}: {error}", temporary.display()))?;
        output
            .write_all(&bytes)
            .and_then(|()| output.write_all(b"\n"))
            .and_then(|()| output.sync_all())
            .map_err(|error| format!("failed to write {}: {error}", temporary.display()))?;
        drop(output);

        (self.publisher)(&temporary, &self.path).map_err(|error| {
            format!(
                "failed to publish settings to {}: {error}",
                self.path.display()
            )
        })?;
        Ok(())
    }

    fn open_lock(&self) -> Result<File, String> {
        let directory = self
            .path
            .parent()
            .ok_or_else(|| "settings path must have a parent directory".to_string())?;
        create_private_dir_all(directory)
            .map_err(|error| format!("failed to create {}: {error}", directory.display()))?;
        private_file_options()
            .create(true)
            .read(true)
            .write(true)
            .truncate(false)
            .open(self.lock_path())
            .map_err(|error| format!("failed to open {}: {error}", self.lock_path().display()))
    }

    fn lock_path(&self) -> PathBuf {
        self.path.with_extension("lock")
    }

    fn temporary_path(&self) -> PathBuf {
        self.path.with_extension("json.tmp")
    }
}

fn private_file_options() -> OpenOptions {
    let mut options = OpenOptions::new();
    #[cfg(unix)]
    options.mode(0o600);
    options
}

fn create_private_dir_all(path: &Path) -> io::Result<()> {
    fs::create_dir_all(path)?;
    #[cfg(unix)]
    fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
    Ok(())
}
