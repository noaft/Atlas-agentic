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

#[cfg(unix)]
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};

pub const SETTINGS_VERSION: u32 = 1;
pub const DEFAULT_DEEPSEEK_BASE_URL: &str = "https://api.deepseek.com";
pub const DEFAULT_DEEPSEEK_MODEL: &str = "deepseek-flash";
pub const DEFAULT_HISTORY_LIMIT: usize = 40;

type SettingsPublisher = dyn Fn(&Path, &Path) -> io::Result<()> + Send + Sync;

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AtlasSettings {
    pub version: u32,
    pub deepseek: DeepSeekSettings,
    pub conversation: ConversationSettings,
    pub workspace: WorkspaceSettings,
}

impl fmt::Debug for AtlasSettings {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AtlasSettings")
            .field("version", &self.version)
            .field("deepseek", &self.deepseek)
            .field("conversation", &self.conversation)
            .field("workspace", &self.workspace)
            .finish()
    }
}

impl Default for AtlasSettings {
    fn default() -> Self {
        Self {
            version: SETTINGS_VERSION,
            deepseek: DeepSeekSettings::default(),
            conversation: ConversationSettings::default(),
            workspace: WorkspaceSettings::default(),
        }
    }
}

impl AtlasSettings {
    pub fn validate(&self) -> Result<(), String> {
        if self.version != SETTINGS_VERSION {
            return Err(format!("unsupported settings version: {}", self.version));
        }
        if self.deepseek.base_url.trim().is_empty() {
            return Err("DeepSeek base URL cannot be blank".to_string());
        }
        if self.deepseek.model.trim().is_empty() {
            return Err("DeepSeek model cannot be blank".to_string());
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
        self.validate()?;
        let mut effective = EffectiveSettings {
            api_key: self.deepseek.api_key.clone(),
            base_url: self.deepseek.base_url.clone(),
            model: self.deepseek.model.clone(),
            history_limit: self.conversation.history_limit,
            default_workspace: self.workspace.default_path.clone(),
            environment_overrides: BTreeSet::new(),
        };

        if let Some(value) = environment_string(&environment, "DEEPSEEK_API_KEY")? {
            effective.api_key = Some(value);
            effective.environment_overrides.insert(SettingField::ApiKey);
        }
        if let Some(value) = environment_string(&environment, "DEEPSEEK_BASE_URL")? {
            effective.base_url = value;
            effective
                .environment_overrides
                .insert(SettingField::BaseUrl);
        }
        if let Some(value) = environment_string(&environment, "DEEPSEEK_MODEL")? {
            effective.model = value;
            effective.environment_overrides.insert(SettingField::Model);
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
    ApiKey,
    BaseUrl,
    Model,
    HistoryLimit,
}

#[derive(Clone, PartialEq, Eq)]
pub struct EffectiveSettings {
    pub api_key: Option<String>,
    pub base_url: String,
    pub model: String,
    pub history_limit: usize,
    pub default_workspace: Option<PathBuf>,
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
            .field("environment_overrides", &self.environment_overrides)
            .finish()
    }
}

impl EffectiveSettings {
    fn validate(&self) -> Result<(), String> {
        if self.base_url.trim().is_empty() {
            return Err("DeepSeek base URL cannot be blank".to_string());
        }
        if self.model.trim().is_empty() {
            return Err("DeepSeek model cannot be blank".to_string());
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
}

impl Default for ConversationSettings {
    fn default() -> Self {
        Self {
            history_limit: DEFAULT_HISTORY_LIMIT,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceSettings {
    pub default_path: Option<PathBuf>,
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
        if !self.path.exists() {
            return Ok(AtlasSettings::default());
        }
        let mut input = String::new();
        File::open(&self.path)
            .and_then(|mut file| file.read_to_string(&mut input))
            .map_err(|error| format!("failed to read {}: {error}", self.path.display()))?;
        let settings: AtlasSettings = serde_json::from_str(&input)
            .map_err(|error| format!("failed to parse {}: {error}", self.path.display()))?;
        settings.validate()?;
        Ok(settings)
    }

    pub fn save(&self, settings: &AtlasSettings) -> Result<(), String> {
        settings.validate()?;
        let lock = self.open_lock()?;
        lock.lock_exclusive()
            .map_err(|error| format!("failed to lock {}: {error}", self.lock_path().display()))?;

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
