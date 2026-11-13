//! Local reusable agent definitions. Tool access is selected anew by each caller.
use cap_fs_ext::{DirExt, FollowSymlinks, OpenOptionsFollowExt};
use cap_std::{
    ambient_authority,
    fs::{Dir, DirBuilder, OpenOptions},
};
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use std::{
    io::{Read, Write},
    path::{Component, Path, PathBuf},
    time::{Duration, Instant},
};

const MAX_FILE: usize = 128 * 1024;
// A lease protects one scope only: 64 definitions per scope bounds both scopes to 128.
const MAX_AGENTS_PER_SCOPE: usize = 64;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentDefinition {
    pub name: String,
    pub display_name: String,
    pub description: String,
    pub system_prompt: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default)]
    pub skills: Vec<String>,
    #[serde(default = "default_max_tool_calls")]
    pub max_tool_calls: usize,
    #[serde(default = "default_timeout")]
    pub timeout_seconds: u64,
}
fn default_max_tool_calls() -> usize {
    40
}
fn default_timeout() -> u64 {
    600
}

impl AgentDefinition {
    pub fn validate(&self) -> Result<(), String> {
        validate_name(&self.name)?;
        bounded_text(&self.display_name, 1024, "display_name")?;
        bounded_text(&self.description, 1024, "description")?;
        bounded_text(&self.system_prompt, 64 * 1024, "system_prompt")?;
        if self.provider.is_some() != self.model.is_some() {
            return Err("provider and model must be specified together".into());
        }
        for value in [&self.provider, &self.model].into_iter().flatten() {
            bounded_text(value, 1024, "provider/model")?;
        }
        if self.max_tool_calls == 0 || self.max_tool_calls > 1000 {
            return Err("max_tool_calls must be 1–1000".into());
        }
        if !(1..=3600).contains(&self.timeout_seconds) {
            return Err("timeout_seconds must be 1–3600".into());
        }
        if self.skills.len() > 128 {
            return Err("at most 128 skills may be selected".into());
        }
        for skill in &self.skills {
            bounded_text(skill, 256, "skill")?;
        }
        if toml::to_string(self).map_err(err)?.len() > MAX_FILE {
            return Err("agent definition exceeds 128 KiB".into());
        }
        Ok(())
    }
}
fn bounded_text(value: &str, limit: usize, field: &str) -> Result<(), String> {
    if value.trim().is_empty() || value.len() > limit || value.contains('\0') {
        Err(format!(
            "{field} must be nonempty and at most {limit} bytes"
        ))
    } else {
        Ok(())
    }
}
fn validate_name(name: &str) -> Result<(), String> {
    if name.is_empty()
        || name.len() > 64
        || !name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
    {
        return Err(
            "agent name must contain 1–64 lowercase ASCII letters, digits or hyphens".into(),
        );
    }
    Ok(())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentInfo {
    pub id: String,
    pub definition: AgentDefinition,
}

#[derive(Clone)]
pub struct AgentCatalog {
    personal: PathBuf,
    project: PathBuf,
}
impl AgentCatalog {
    pub fn new(personal_root: PathBuf, workspace: PathBuf) -> Self {
        Self {
            personal: personal_root,
            project: workspace.join(".agents/agents"),
        }
    }
    pub fn for_settings(settings: &crate::settings::SettingsStore, workspace: &Path) -> Self {
        Self::new(settings.directory().join("agents"), workspace.to_path_buf())
    }
    fn root(&self, scope: &str) -> Result<&Path, String> {
        match scope {
            "personal" => Ok(&self.personal),
            "project" => Ok(&self.project),
            _ => Err("agent scope must be personal or project".into()),
        }
    }
    pub fn list(&self) -> Result<Vec<AgentInfo>, String> {
        let mut result = Vec::new();
        for scope in ["personal", "project"] {
            let Some(root) = open_root(self.root(scope)?, false)? else {
                continue;
            };
            result.extend(list_scope(&root, scope)?);
        }
        result.sort_by(|a, b| a.id.cmp(&b.id));
        Ok(result)
    }
    pub fn load(&self, id: &str) -> Result<AgentDefinition, String> {
        let (scope, name) = id
            .split_once(':')
            .ok_or("agent ID must be personal:<name> or project:<name>")?;
        validate_name(name)?;
        let root = open_root(self.root(scope)?, false)?.ok_or("agent storage does not exist")?;
        read_definition(&root, name)
    }
    pub fn create(&self, scope: &str, definition: AgentDefinition) -> Result<AgentInfo, String> {
        let path = self.root(scope)?;
        definition.validate()?;
        let bytes = toml::to_string(&definition).map_err(err)?.into_bytes();
        if bytes.len() > MAX_FILE {
            return Err("agent definition exceeds 128 KiB".into());
        }
        let root = open_root(path, true)?.ok_or("agent storage does not exist")?;
        let _lease = lease(&root)?;
        if list_scope(&root, scope)?.len() >= MAX_AGENTS_PER_SCOPE {
            return Err("agent scope is full (64 definitions per scope; 128 total)".into());
        }
        let temporary = format!(".agent-{}.tmp", uuid::Uuid::new_v4());
        let mut options = OpenOptions::new();
        options.follow(FollowSymlinks::No);
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use cap_std::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = root.open_with(&temporary, &options).map_err(err)?;
        let outcome = (|| {
            file.write_all(&bytes).map_err(err)?;
            file.sync_all().map_err(err)?;
            let current = open_root(path, false)?.ok_or("agent storage changed")?;
            let original =
                same_file::Handle::from_file(root.try_clone().map_err(err)?.into_std_file())
                    .map_err(err)?;
            let current = same_file::Handle::from_file(current.into_std_file()).map_err(err)?;
            if original != current {
                return Err("agent storage changed".into());
            }
            root.hard_link(&temporary, &root, format!("{}.toml", definition.name))
                .map_err(err)
        })();
        drop(file);
        let _ = root.remove_file(&temporary);
        outcome?;
        Ok(AgentInfo {
            id: format!("{scope}:{}", definition.name),
            definition,
        })
    }
}
fn list_scope(root: &Dir, scope: &str) -> Result<Vec<AgentInfo>, String> {
    let mut result = Vec::new();
    for entry in root.entries().map_err(err)? {
        let entry = entry.map_err(err)?;
        let name = entry.file_name();
        let Some(name) = name.to_str().and_then(|n| n.strip_suffix(".toml")) else {
            continue;
        };
        validate_name(name)?;
        if result.len() == MAX_AGENTS_PER_SCOPE {
            return Err("agent scope exceeds 64 definitions (128 total)".into());
        }
        result.push(AgentInfo {
            id: format!("{scope}:{name}"),
            definition: read_definition(root, name)?,
        });
    }
    Ok(result)
}
fn read_definition(root: &Dir, name: &str) -> Result<AgentDefinition, String> {
    let path = format!("{name}.toml");
    if !root.symlink_metadata(&path).map_err(err)?.is_file() {
        return Err("agent definition must be a regular file".into());
    }
    let mut options = OpenOptions::new();
    options.follow(FollowSymlinks::No);
    options.read(true);
    #[cfg(unix)]
    {
        use cap_std::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    let file = root.open_with(&path, &options).map_err(err)?;
    if !file.metadata().map_err(err)?.is_file() {
        return Err("agent definition must be a regular file".into());
    }
    let mut bytes = Vec::new();
    file.take((MAX_FILE + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(err)?;
    if bytes.len() > MAX_FILE {
        return Err("agent definition exceeds 128 KiB".into());
    }
    let text = std::str::from_utf8(&bytes).map_err(|_| "agent definition must be UTF-8")?;
    let definition: AgentDefinition =
        toml::from_str(text).map_err(|_| "invalid agent definition TOML".to_string())?;
    definition.validate()?;
    if definition.name != name {
        return Err("agent definition name does not match its filename".into());
    }
    Ok(definition)
}
fn open_root(path: &Path, create: bool) -> Result<Option<Dir>, String> {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir().map_err(err)?.join(path)
    };
    let mut components = absolute.components();
    let first = components.next().ok_or("invalid agent root")?;
    #[cfg(unix)]
    let mut current = {
        if first != Component::RootDir {
            return Err("invalid agent root".into());
        }
        Dir::open_ambient_dir("/", ambient_authority()).map_err(err)?
    };
    #[cfg(windows)]
    let mut current = {
        let mut anchor = PathBuf::from(first.as_os_str());
        if components.clone().next() == Some(Component::RootDir) {
            anchor.push("\\");
            components.next();
        }
        Dir::open_ambient_dir(anchor, ambient_authority()).map_err(err)?
    };
    for component in components {
        let Component::Normal(name) = component else {
            return Err("unsafe agent root".into());
        };
        match current.symlink_metadata(name) {
            Ok(metadata) if metadata.is_dir() => {}
            Ok(_) => return Err("agent root contains a symlink or non-directory".into()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                if !create {
                    return Ok(None);
                }
                let mut builder = DirBuilder::new();
                #[cfg(unix)]
                {
                    use cap_std::fs::DirBuilderExt;
                    builder.mode(0o700);
                }
                match current.create_dir_with(name, &builder) {
                    Ok(()) => {}
                    Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
                    Err(e) => return Err(err(e)),
                }
                if !current.symlink_metadata(name).map_err(err)?.is_dir() {
                    return Err("unsafe agent root".into());
                }
            }
            Err(e) => return Err(err(e)),
        }
        current = open_child(&current, name)?;
    }
    Ok(Some(current))
}
fn open_child(parent: &Dir, name: &std::ffi::OsStr) -> Result<Dir, String> {
    let directory = parent.open_dir_nofollow(name).map_err(err)?;
    if !directory.dir_metadata().map_err(err)?.is_dir() {
        return Err("agent root is not a directory".into());
    }
    Ok(directory)
}
fn lease(root: &Dir) -> Result<std::fs::File, String> {
    let mut options = OpenOptions::new();
    options.follow(FollowSymlinks::No);
    options.read(true).write(true).create(true);
    #[cfg(unix)]
    {
        use cap_std::fs::OpenOptionsExt;
        options
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    if let Ok(metadata) = root.symlink_metadata(".agents.lock") {
        if !metadata.is_file() {
            return Err("unsafe agent lock".into());
        }
    }
    let file = root
        .open_with(".agents.lock", &options)
        .map_err(err)?
        .into_std();
    if !file.metadata().map_err(err)?.is_file() {
        return Err("unsafe agent lock".into());
    }
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        match file.try_lock_exclusive() {
            Ok(()) => return Ok(file),
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock && Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(10))
            }
            Err(_) => return Err("agent storage is busy".into()),
        }
    }
}
fn err(error: impl std::fmt::Display) -> String {
    format!("agent storage: {error}")
}
#[cfg(test)]
mod tests {
    use super::*;

    fn definition() -> AgentDefinition {
        serde_json::from_value(serde_json::json!({
            "name":"reviewer", "display_name":"Reviewer", "description":"Review changes",
            "system_prompt":"Review the supplied changes carefully."
        }))
        .unwrap()
    }

    #[test]
    fn creates_persistent_scoped_definitions_without_overwriting() {
        let dir = tempfile::tempdir().unwrap();
        let catalog = AgentCatalog::new(dir.path().join("personal"), dir.path().join("workspace"));
        let personal = catalog.create("personal", definition()).unwrap();
        assert_eq!(personal.id, "personal:reviewer");
        assert_eq!(catalog.load(&personal.id).unwrap().max_tool_calls, 40);
        assert_eq!(catalog.load(&personal.id).unwrap().timeout_seconds, 600);
        assert!(catalog.create("personal", definition()).is_err());
        catalog.create("project", definition()).unwrap();
        assert!(dir
            .path()
            .join("workspace/.agents/agents/reviewer.toml")
            .is_file());
        assert_eq!(catalog.list().unwrap().len(), 2);
        let reloaded = AgentCatalog::new(dir.path().join("personal"), dir.path().join("workspace"));
        assert_eq!(
            reloaded.load("personal:reviewer").unwrap().system_prompt,
            definition().system_prompt
        );
    }

    #[test]
    fn validation_bounds_serialized_toml() {
        let mut value = definition();
        value.system_prompt = "\u{1}".repeat(65536);
        assert!(value.validate().is_err());
    }

    #[test]
    fn validates_bounds_unknown_fields_and_provider_pairs() {
        let dir = tempfile::tempdir().unwrap();
        let catalog = AgentCatalog::new(dir.path().join("p"), dir.path().join("w"));
        for name in ["", "../escape", "UPPER", "with space", &"x".repeat(65)] {
            let mut value = definition();
            value.name = name.into();
            assert!(catalog.create("personal", value).is_err());
        }
        for timeout in [0, 3601] {
            let mut value = definition();
            value.timeout_seconds = timeout;
            assert!(catalog.create("personal", value).is_err());
        }
        let mut value = definition();
        value.provider = Some("openai".into());
        assert!(catalog.create("personal", value).is_err());
        let mut value = definition();
        value.system_prompt = "x".repeat(65537);
        assert!(catalog.create("personal", value).is_err());
        let mut value = definition();
        value.description = "x".repeat(1025);
        assert!(catalog.create("personal", value).is_err());
        let mut json = serde_json::to_value(definition()).unwrap();
        json["tools"] = serde_json::json!([]);
        assert!(serde_json::from_value::<AgentDefinition>(json).is_err());
        assert!(catalog.load("../escape").is_err());
        assert!(catalog.list().unwrap().is_empty());
    }

    #[test]
    fn bounded_listing_and_invalid_files_fail_closed() {
        let dir = tempfile::tempdir().unwrap();
        let catalog = AgentCatalog::new(dir.path().join("p"), dir.path().join("w"));
        for scope in ["personal", "project"] {
            for i in 0..64 {
                let mut value = definition();
                value.name = format!("agent-{i}");
                catalog.create(scope, value).unwrap();
            }
            let mut value = definition();
            value.name = "overflow".into();
            assert!(catalog.create(scope, value).is_err());
        }
        assert_eq!(catalog.list().unwrap().len(), 128);
        std::fs::write(dir.path().join("p/agent-0.toml"), "tools = []").unwrap();
        assert!(catalog.load("personal:agent-0").is_err());
    }

    #[test]
    fn project_creation_does_not_inspect_or_create_personal_storage() {
        let dir = tempfile::tempdir().unwrap();
        let personal = dir.path().join("personal");
        std::fs::write(&personal, "unavailable personal storage").unwrap();
        let catalog = AgentCatalog::new(personal, dir.path().join("workspace"));
        catalog.create("project", definition()).unwrap();
        assert_eq!(catalog.load("project:reviewer").unwrap().name, "reviewer");
    }

    #[cfg(unix)]
    #[test]
    fn refuses_symlinks_and_publishes_private_files() {
        use std::os::unix::fs::{symlink, PermissionsExt};
        let dir = tempfile::tempdir().unwrap();
        let catalog = AgentCatalog::new(dir.path().join("p"), dir.path().join("w"));
        catalog.create("personal", definition()).unwrap();
        assert_eq!(
            std::fs::metadata(dir.path().join("p/reviewer.toml"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
        symlink(
            dir.path().join("p/reviewer.toml"),
            dir.path().join("p/alias.toml"),
        )
        .unwrap();
        assert!(catalog.load("personal:alias").is_err());
        assert!(catalog.list().is_err());
        symlink(dir.path().join("p"), dir.path().join("linked")).unwrap();
        let linked = AgentCatalog::new(dir.path().join("linked"), dir.path().join("w"));
        assert!(linked.create("personal", definition()).is_err());
    }
}
