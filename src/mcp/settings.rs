//! Add-only MCP Settings storage. Inspection is deliberately separate from persistence.
use super::{McpConfig, ServerConfig};
use cap_std::{
    ambient_authority,
    fs::{Dir, OpenOptions},
};
use fs2::FileExt;
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    ffi::{OsStr, OsString},
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

const MAX_BYTES: usize = 1024 * 1024;

#[derive(Clone)]
pub struct McpConfigStore {
    path: PathBuf,
}
pub struct McpConfigSnapshot {
    pub path: PathBuf,
    pub fingerprint: String,
    pub config: McpConfig,
}
impl McpConfigStore {
    pub fn new(path: PathBuf) -> Result<Self, String> {
        let path =
            std::path::absolute(path).map_err(|_| "Could not resolve MCP configuration path")?;
        if path.file_name().is_none() {
            return Err("MCP configuration needs a file name".into());
        }
        Ok(Self { path })
    }
    pub fn from_environment() -> Result<Self, String> {
        Self::new(McpConfig::path_from_environment()?)
    }
    pub fn path(&self) -> &Path {
        &self.path
    }
    fn name(&self) -> &OsStr {
        self.path
            .file_name()
            .expect("validated configuration file name")
    }
    fn fingerprint(&self, bytes: Option<&str>) -> String {
        let mut hash = Sha256::new();
        hash.update(self.path.as_os_str().as_encoded_bytes());
        hash.update([0, u8::from(bytes.is_some())]);
        if let Some(bytes) = bytes {
            hash.update(bytes.as_bytes());
        }
        format!("{:x}", hash.finalize())
    }
    fn snapshot(&self, bytes: Option<String>) -> Result<McpConfigSnapshot, String> {
        let config = match &bytes {
            Some(bytes) => McpConfig::from_toml(bytes)?,
            None => McpConfig::default(),
        };
        Ok(McpConfigSnapshot {
            path: self.path.clone(),
            fingerprint: self.fingerprint(bytes.as_deref()),
            config,
        })
    }
    pub fn load(&self) -> Result<McpConfigSnapshot, String> {
        let dir = match Dir::open_ambient_dir(
            self.path.parent().expect("absolute path"),
            ambient_authority(),
        ) {
            Ok(dir) => dir,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return self.snapshot(None)
            }
            Err(_) => return Err("Could not open MCP configuration folder".into()),
        };
        self.snapshot(read(&dir, self.name())?)
    }
    pub fn add(
        &self,
        expected: &str,
        name: &str,
        server: ServerConfig,
    ) -> Result<McpConfigSnapshot, String> {
        if expected.len() != 64 || !expected.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err("Invalid MCP configuration fingerprint; reload configuration".into());
        }
        let addition = toml::to_string(&McpConfig {
            servers: BTreeMap::from([(name.into(), server)]),
        })
        .map_err(|_| "Invalid MCP server configuration")?;
        McpConfig::from_toml(&addition)?;
        let parent = self.path.parent().expect("absolute path");
        let mut builder = fs::DirBuilder::new();
        builder.recursive(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            builder.mode(0o700);
        }
        builder
            .create(parent)
            .map_err(|_| "Could not create MCP configuration folder")?;
        let dir = Dir::open_ambient_dir(parent, ambient_authority())
            .map_err(|_| "Could not open MCP configuration folder")?;
        let mut lock_name = self.name().to_os_string();
        lock_name.push(".lock");
        reject_special(&dir, &lock_name)?;
        let mut lock_options = options();
        lock_options
            .create(true)
            .read(true)
            .write(true)
            .truncate(false);
        let lock = dir
            .open_with(&lock_name, &lock_options)
            .map_err(|_| "Could not open MCP configuration lock")?
            .into_std();
        if !lock
            .metadata()
            .map_err(|_| "Could not inspect MCP configuration lock")?
            .is_file()
        {
            return Err("MCP configuration lock must be a regular file".into());
        }
        let deadline = Instant::now() + Duration::from_millis(1500);
        loop {
            match lock.try_lock_exclusive() {
                Ok(()) => break,
                Err(error)
                    if error.kind() == std::io::ErrorKind::WouldBlock
                        && Instant::now() < deadline =>
                {
                    std::thread::sleep(Duration::from_millis(10))
                }
                Err(_) => return Err("MCP configuration is busy; retry shortly".into()),
            }
        }
        let original = read(&dir, self.name())?;
        let snapshot = self.snapshot(original.clone())?;
        if snapshot.fingerprint != expected {
            return Err("MCP configuration changed; reload configuration before saving".into());
        }
        if snapshot.config.servers.contains_key(name) {
            return Err("An MCP server with this name already exists".into());
        }
        let content = format!("{}\n\n{}", original.as_deref().unwrap_or(""), addition);
        if content.len() > MAX_BYTES {
            return Err("MCP configuration exceeds 1 MiB".into());
        }
        self.snapshot(Some(content.clone()))?;
        let mut temporary = OsString::from(".atlas-mcp-");
        temporary.push(uuid::Uuid::new_v4().to_string());
        temporary.push(".tmp");
        let result = (|| {
            let mut output = options();
            output.write(true).create_new(true);
            let mut file = dir
                .open_with(&temporary, &output)
                .map_err(|_| "Could not stage MCP configuration")?;
            file.write_all(content.as_bytes())
                .and_then(|()| file.sync_all())
                .map_err(|_| "Could not write MCP configuration")?;
            drop(file);
            // A non-cooperating writer cannot redirect this publication to a symlink.
            reject_special(&dir, self.name())?;
            if read(&dir, self.name())? != original {
                return Err(
                    "MCP configuration changed before publication; reload configuration".into(),
                );
            }
            dir.rename(&temporary, &dir, self.name())
                .map_err(|_| "Could not publish MCP configuration")?;
            if read(&dir, self.name())?.as_deref() != Some(content.as_str()) {
                return Err("MCP configuration could not be verified after saving".into());
            }
            self.snapshot(Some(content))
        })();
        let _ = dir.remove_file(&temporary);
        result
    }
}
fn options() -> OpenOptions {
    let mut options = OpenOptions::new();
    #[cfg(unix)]
    {
        use cap_std::fs::OpenOptionsExt;
        options
            .mode(0o600)
            .custom_flags(libc::O_NONBLOCK | libc::O_NOFOLLOW);
    }
    #[cfg(windows)]
    {
        use cap_std::fs::OpenOptionsExt;
        options.custom_flags(windows_sys::Win32::Storage::FileSystem::FILE_FLAG_OPEN_REPARSE_POINT);
    }
    options
}
fn reject_special(dir: &Dir, name: &OsStr) -> Result<(), String> {
    match dir.symlink_metadata(name) {
        Ok(meta) if !meta.is_file() || meta.file_type().is_symlink() => {
            Err("MCP configuration and lock must be regular files, not symlinks".into())
        }
        Ok(_) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(_) => Err("Could not inspect MCP configuration".into()),
    }
}
fn read(dir: &Dir, name: &OsStr) -> Result<Option<String>, String> {
    reject_special(dir, name)?;
    let mut open = options();
    open.read(true);
    let file = match dir.open_with(name, &open) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err("Could not read MCP configuration".into()),
    };
    if !file
        .metadata()
        .map_err(|_| "Could not inspect MCP configuration")?
        .is_file()
    {
        return Err("MCP configuration must be a regular file".into());
    }
    let mut bytes = Vec::new();
    file.take((MAX_BYTES + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|_| "Could not read MCP configuration")?;
    if bytes.len() > MAX_BYTES {
        return Err("MCP configuration exceeds 1 MiB".into());
    }
    String::from_utf8(bytes)
        .map(Some)
        .map_err(|_| "MCP configuration must be UTF-8".into())
}
