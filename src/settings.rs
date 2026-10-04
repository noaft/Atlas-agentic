use std::{
    env, fmt, fs,
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AtlasSettings {
    pub version: u32,
    pub deepseek: DeepSeekSettings,
    pub conversation: ConversationSettings,
    pub workspace: WorkspaceSettings,
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
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeepSeekSettings {
    pub api_key: Option<String>,
    pub base_url: String,
    pub model: String,
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
