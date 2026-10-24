use super::*;
use cap_std::{
    ambient_authority,
    fs::{Dir, DirBuilder, OpenOptions},
};
use fs2::FileExt;
use std::{
    io::{Read, Write},
    path::{Component, Path},
};
const MANIFEST: &str = ".atlas-skill.json";
type SkillFiles = BTreeMap<String, Vec<u8>>;
pub(crate) struct FrozenSkill {
    pub info: SkillInfo,
    pub directory: PathBuf,
    pub resources: Vec<String>,
    pub texts: BTreeMap<String, String>,
}
type SkillSnapshot = Vec<FrozenSkill>;
#[derive(Clone)]
pub struct SkillStore {
    personal: PathBuf,
    project: PathBuf,
    fallbacks: Vec<PathBuf>,
}
#[derive(Serialize, Deserialize)]
struct Manifest {
    enabled: bool,
    source: Option<SkillSource>,
}
impl Default for Manifest {
    fn default() -> Self {
        Self {
            enabled: true,
            source: None,
        }
    }
}
impl SkillStore {
    pub fn new(personal_root: PathBuf, workspace: PathBuf) -> Self {
        Self {
            personal: personal_root,
            fallbacks: Vec::new(),
            project: workspace.join(".agents/skills"),
        }
    }
    pub fn for_settings(
        settings: &crate::settings::SettingsStore,
        workspace: PathBuf,
    ) -> Result<Self, String> {
        let default = crate::settings::SettingsStore::from_user_config()?;
        let isolated = std::env::var_os("ATLAS_CONFIG_DIR").is_some()
            || settings.directory() != default.directory();
        let mut store = Self::new(
            if isolated {
                settings.directory().join("skills")
            } else {
                personal_skills_root()?
            },
            workspace,
        );
        if !isolated {
            let home = directories::BaseDirs::new().ok_or("could not resolve user home")?;
            for fallback in [
                home.home_dir().join(".agents/skills"),
                settings.directory().join("skills"),
            ] {
                if fallback != store.personal && !store.fallbacks.contains(&fallback) {
                    store.fallbacks.push(fallback);
                }
            }
        }
        Ok(store)
    }
    fn existing_path(&self, scope: SkillScope, name: &str) -> Result<&Path, String> {
        let primary = self.path(scope);
        if scope == SkillScope::Personal && !entry_exists(&primary.join(name))? {
            for legacy in &self.fallbacks {
                if entry_exists(&legacy.join(name))? {
                    return Ok(legacy);
                }
            }
        }
        Ok(primary)
    }
    fn path(&self, scope: SkillScope) -> &Path {
        match scope {
            SkillScope::Personal => &self.personal,
            SkillScope::Project => &self.project,
        }
    }
    pub fn list(&self) -> Result<SkillCatalog, String> {
        let mut catalog = SkillCatalog {
            skills: vec![],
            issues: vec![],
        };
        let mut enabled_bytes = 0;
        for (scope, path) in std::iter::once((SkillScope::Personal, self.personal.as_path()))
            .chain(
                self.fallbacks
                    .iter()
                    .map(|p| (SkillScope::Personal, p.as_path())),
            )
            .chain(std::iter::once((
                SkillScope::Project,
                self.project.as_path(),
            )))
        {
            match path.try_exists() {
                Ok(true) => {}
                Ok(false) => continue,
                Err(error) => {
                    issue(&mut catalog, format!("{}: {}", scope.label(), err(error)));
                    continue;
                }
            }
            let result = (|| {
                let root = open_root(path, false)?;
                let _lock = lock(&root)?;
                for error in recover(&root, scope) {
                    issue(&mut catalog, error);
                }
                let mut names = root
                    .entries()
                    .map_err(err)?
                    .take(1025)
                    .collect::<Result<Vec<_>, _>>()
                    .map_err(err)?;
                if names.len() > 1024 {
                    issue(
                        &mut catalog,
                        "skill root scan limited to 1024 entries".into(),
                    );
                    names.truncate(1024);
                }
                names.sort_by_key(|e| e.file_name());
                for entry in names {
                    let name = entry.file_name();
                    let Some(name) = name.to_str() else {
                        issue(&mut catalog, "non-UTF-8 skill folder".into());
                        continue;
                    };
                    if name.starts_with('.') {
                        continue;
                    }
                    if catalog.skills.len() >= 512 {
                        issue(&mut catalog, "skill catalog exceeds 512 entries".into());
                        break;
                    }
                    if scope == SkillScope::Personal && self.existing_path(scope, name)? != path {
                        continue;
                    }
                    if catalog
                        .skills
                        .iter()
                        .any(|i| i.id == format!("{}:{name}", scope.label()))
                    {
                        continue;
                    }
                    match load(&root, name, scope) {
                        Ok((info, files)) => {
                            if info.enabled {
                                enabled_bytes += files.values().map(Vec::len).sum::<usize>();
                            }
                            catalog.skills.push(info)
                        }
                        Err(error) => {
                            issue(&mut catalog, format!("{}:{name}: {error}", scope.label()))
                        }
                    }
                }
                Ok::<_, String>(())
            })();
            if let Err(error) = result {
                issue(&mut catalog, format!("{}: {error}", scope.label()));
            }
        }
        let mut count = 0;
        let mut metadata_bytes = 0;
        for skill in &catalog.skills {
            if !skill.enabled {
                continue;
            }
            count += 1;
            metadata_bytes += model_metadata(skill).to_string().len() + 1;
        }
        if count > 64 || metadata_bytes > 64 * 1024 {
            issue(&mut catalog,"Runtime skill catalog is bounded to 64 enabled skills and 64 KiB metadata; excess skills are omitted".into());
        }
        if enabled_bytes > 32 * 1024 * 1024 {
            issue(&mut catalog,"Runtime skill text snapshots are bounded to 32 MiB; packages exceeding the remaining budget are omitted".into());
        }
        Ok(catalog)
    }
    pub fn read(&self, id: &str, resource: Option<&str>) -> Result<SkillDocument, String> {
        let (scope, name) = parse_id(id)?;
        let selected_root = self.existing_path(scope, name)?;
        let root = open_root(selected_root, false)?;
        let _lock = lock(&root)?;
        recover(&root, scope);
        let (skill, files) = load(&root, name, scope)?;
        let resource = resource.unwrap_or("SKILL.md");
        validate_path(resource)?;
        if resource == MANIFEST {
            return Err("skill manifest is private".into());
        }
        let bytes = files.get(resource).ok_or("skill resource not found")?;
        let content = String::from_utf8(bytes.clone())
            .map_err(|_| "skill resource must be UTF-8 text".to_string())?;
        Ok(SkillDocument {
            fingerprint: document_fingerprint(selected_root, &files)?,
            skill,
            content,
            files: files.keys().cloned().collect(),
        })
    }
    /// Snapshot enabled package text for a request. Later filesystem mutations cannot
    /// substitute instructions or supporting resources into that running request.
    pub(crate) fn snapshot(&self) -> Result<SkillSnapshot, String> {
        let mut result = Vec::new();
        let mut bytes = 0;
        let mut metadata_bytes = 0;
        for info in self.list()?.skills.into_iter().filter(|i| i.enabled) {
            let loaded = (|| {
                let (scope, name) = parse_id(&info.id)?;
                let selected_root = self.existing_path(scope, name)?;
                let root = open_root(selected_root, false)?;
                let _lock = lock(&root)?;
                let (current, files) = load(&root, name, scope)?;
                let directory = selected_root.join(name);
                let directory = if directory.is_absolute() {
                    directory
                } else {
                    std::env::current_dir().map_err(err)?.join(directory)
                };
                Ok::<_, String>((current, files, directory))
            })();
            let Ok((current, files, directory)) = loaded else {
                continue;
            };
            if !current.enabled {
                continue;
            }
            let size = files.values().map(Vec::len).sum::<usize>();
            let metadata_size = model_metadata(&current).to_string().len() + 1;
            if result.len() >= 64
                || bytes + size > 32 * 1024 * 1024
                || metadata_bytes + metadata_size > 64 * 1024
            {
                continue;
            }
            bytes += size;
            metadata_bytes += metadata_size;
            let resources = files.keys().cloned().collect();
            let mut texts = BTreeMap::new();
            for (path, bytes) in files {
                if let Ok(text) = String::from_utf8(bytes) {
                    texts.insert(path, text);
                }
            }
            result.push(FrozenSkill {
                info: current,
                directory,
                resources,
                texts,
            });
        }
        Ok(result)
    }
    pub fn install(
        &self,
        package: SkillPackage,
        scope: SkillScope,
        replace: bool,
    ) -> Result<SkillInfo, String> {
        if let Some(source) = &package.source {
            validate_source(source)?;
        }
        validate_files(&package.files)?;
        let metadata = parse_skill(
            std::str::from_utf8(
                package
                    .files
                    .get("SKILL.md")
                    .ok_or("package requires SKILL.md")?,
            )
            .map_err(|_| "SKILL.md must be UTF-8")?,
        )?;
        let target = if replace {
            self.existing_path(scope, &metadata.name)?
        } else {
            self.path(scope)
        };
        let root = open_root(target, true)?;
        let _lock = lock(&root)?;
        recover(&root, scope);
        if !replace && scope == SkillScope::Personal {
            for legacy in &self.fallbacks {
                if entry_exists(&legacy.join(&metadata.name))? {
                    return Err("skill is already installed".into());
                }
            }
        }
        publish(&root, package, metadata, scope, replace)
    }
    pub fn create(&self, input: SkillDraftInput, scope: SkillScope) -> Result<SkillInfo, String> {
        self.install(input.package()?, scope, false)
    }
    pub fn update_document(
        &self,
        id: &str,
        content: &str,
        expected: &str,
    ) -> Result<SkillInfo, String> {
        let (scope, name) = parse_id(id)?;
        let metadata = parse_skill(content)?;
        if metadata.name != name {
            return Err("editing cannot rename a skill".into());
        }
        let selected_root = self.existing_path(scope, name)?;
        let root = open_root(selected_root, false)?;
        let _lock = lock(&root)?;
        recover(&root, scope);
        let (_, mut files) = load(&root, name, scope)?;
        if document_fingerprint(selected_root, &files)? != expected {
            return Err("skill changed since it was read; reload before saving".into());
        }
        files.insert("SKILL.md".into(), content.as_bytes().to_vec());
        validate_files(&files)?;
        publish(
            &root,
            SkillPackage {
                source: None,
                files,
            },
            metadata,
            scope,
            true,
        )
    }
    pub fn set_enabled(&self, id: &str, enabled: bool) -> Result<SkillInfo, String> {
        let (scope, name) = parse_id(id)?;
        let selected_root = self.existing_path(scope, name)?;
        let root = open_root(selected_root, false)?;
        let _lock = lock(&root)?;
        recover(&root, scope);
        let (mut info, _) = load(&root, name, scope)?;
        let dir = root.open_dir(name).map_err(err)?;
        let manifest = Manifest {
            enabled,
            source: info.source.clone(),
        };
        let temporary = format!(".manifest-{}", uuid::Uuid::new_v4());
        write_new(
            &dir,
            &temporary,
            &serde_json::to_vec(&manifest).map_err(err)?,
        )?;
        if let Err(error) = dir.rename(&temporary, &dir, MANIFEST) {
            let _ = dir.remove_file(&temporary);
            return Err(err(error));
        }
        info.enabled = enabled;
        Ok(info)
    }
}
fn publish(
    root: &Dir,
    package: SkillPackage,
    metadata: SkillMetadata,
    scope: SkillScope,
    replace: bool,
) -> Result<SkillInfo, String> {
    let exists = root.symlink_metadata(&metadata.name).is_ok();
    if exists && !replace {
        return Err("skill is already installed; use explicit Update".into());
    }
    let enabled = if exists {
        load(root, &metadata.name, scope)?.0.enabled
    } else {
        true
    };
    let manifest = Manifest {
        enabled,
        source: package.source,
    };
    let staging = format!(".install-{}", uuid::Uuid::new_v4());
    private_dir(root, &staging)?;
    let stage = root.open_dir(&staging).map_err(err)?;
    let prepared = (|| {
        for (path, bytes) in &package.files {
            if let Some(parent) = Path::new(path).parent() {
                if !parent.as_os_str().is_empty() {
                    create_subdirs(&stage, parent)?;
                }
            }
            write_new(&stage, path, bytes)?;
        }
        write_new(
            &stage,
            MANIFEST,
            &serde_json::to_vec(&manifest).map_err(err)?,
        )?;
        Ok::<_, String>(())
    })();
    if let Err(error) = prepared {
        let _ = root.remove_dir_all(&staging);
        return Err(error);
    }
    let backup = format!(".previous-{}", uuid::Uuid::new_v4());
    if exists {
        if let Err(error) = root.rename(&metadata.name, root, &backup) {
            let _ = root.remove_dir_all(&staging);
            return Err(err(error));
        }
    }
    if let Err(error) = root.rename(&staging, root, &metadata.name) {
        if exists {
            let _ = root.rename(&backup, root, &metadata.name);
        }
        let _ = root.remove_dir_all(&staging);
        return Err(err(error));
    }
    if exists {
        let _ = root.remove_dir_all(&backup);
    }
    Ok(info(metadata, scope, manifest))
}

// Editor tokens bind the package to the selected storage root. A newly installed
// primary skill must not redirect a draft that was read from the legacy root.
fn document_fingerprint(root: &Path, files: &SkillFiles) -> Result<String, String> {
    use sha2::{Digest, Sha256};
    let identity = std::fs::canonicalize(root).map_err(err)?;
    let identity = identity.as_os_str().as_encoded_bytes();
    let mut hash = Sha256::new();
    hash.update((identity.len() as u64).to_be_bytes());
    hash.update(identity);
    hash.update(fingerprint(files).as_bytes());
    Ok(format!("{:x}", hash.finalize()))
}

fn entry_exists(path: &Path) -> Result<bool, String> {
    match std::fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(err(error)),
    }
}

fn issue(catalog: &mut SkillCatalog, message: String) {
    if catalog.issues.len() < 64 {
        catalog.issues.push(message.chars().take(300).collect());
    }
}
fn err(error: impl std::fmt::Display) -> String {
    format!("skill storage: {error}")
}
fn parse_id(id: &str) -> Result<(SkillScope, &str), String> {
    let (scope, name) = id.split_once(':').ok_or("invalid scoped skill ID")?;
    validate_name(name)?;
    let scope = match scope {
        "personal" => SkillScope::Personal,
        "project" => SkillScope::Project,
        _ => return Err("invalid skill scope".into()),
    };
    Ok((scope, name))
}
fn info(metadata: SkillMetadata, scope: SkillScope, manifest: Manifest) -> SkillInfo {
    SkillInfo {
        id: format!("{}:{}", scope.label(), metadata.name),
        name: metadata.name,
        title: metadata.title,
        description: metadata.description,
        scope,
        enabled: manifest.enabled,
        source: manifest.source,
    }
}
fn load(root: &Dir, name: &str, scope: SkillScope) -> Result<(SkillInfo, SkillFiles), String> {
    validate_name(name)?;
    if !root.symlink_metadata(name).map_err(err)?.is_dir() {
        return Err("skill folder must be a directory, not a symlink".into());
    }
    let dir = root.open_dir(name).map_err(err)?;
    load_directory(&dir, name, scope)
}
fn load_directory(
    dir: &Dir,
    name: &str,
    scope: SkillScope,
) -> Result<(SkillInfo, SkillFiles), String> {
    let mut files = BTreeMap::new();
    let mut total = 0;
    let mut entries = 0;
    walk(dir, "", &mut files, &mut total, &mut entries)?;
    let manifest = if let Some(bytes) = files.remove(MANIFEST) {
        let m: Manifest = serde_json::from_slice(&bytes).map_err(|_| "invalid skill manifest")?;
        if let Some(source) = &m.source {
            validate_source(source)?;
        }
        m
    } else {
        Manifest::default()
    };
    validate_files(&files)?;
    let content = std::str::from_utf8(files.get("SKILL.md").ok_or("missing SKILL.md")?)
        .map_err(|_| "SKILL.md must be UTF-8")?;
    let metadata = parse_skill(content)?;
    if metadata.name != name {
        return Err("skill folder must match metadata name".into());
    }
    Ok((info(metadata, scope, manifest), files))
}
pub(super) fn validate_files(files: &BTreeMap<String, Vec<u8>>) -> Result<(), String> {
    if files.is_empty() || files.len() > MAX_FILES {
        return Err("skill exceeds 256 files".into());
    }
    let mut total = 0;
    let mut directories = std::collections::BTreeSet::new();
    for (path, bytes) in files {
        validate_path(path)?;
        let mut parent = Path::new(path).parent();
        while let Some(directory) = parent {
            if directory.as_os_str().is_empty() {
                break;
            }
            directories.insert(directory);
            parent = directory.parent();
        }
        if directories.len() + files.len() + 1 > 1024 {
            return Err("skill exceeds 1024 filesystem entries".into());
        }
        if path.split('/').any(|part| part.starts_with(".atlas-skill")) {
            return Err("reserved skill manifest filename".into());
        }
        if bytes.len() > MAX_FILE_BYTES {
            return Err("skill file exceeds 256 KiB".into());
        }
        total += bytes.len();
        if total > MAX_PACKAGE_BYTES {
            return Err("skill package exceeds 8 MiB".into());
        }
    }
    Ok(())
}
pub(super) fn walk(
    dir: &Dir,
    prefix: &str,
    files: &mut BTreeMap<String, Vec<u8>>,
    total: &mut usize,
    entries: &mut usize,
) -> Result<(), String> {
    if prefix.split('/').count() > 32 {
        return Err("skill folder nesting exceeds 32 levels".into());
    }
    for entry in dir.entries().map_err(err)? {
        *entries += 1;
        if *entries > 1024 {
            return Err("skill exceeds 1024 filesystem entries".into());
        }
        let entry = entry.map_err(err)?;
        let name = entry
            .file_name()
            .into_string()
            .map_err(|_| "skill filenames must be UTF-8")?;
        let path = if prefix.is_empty() {
            name.clone()
        } else {
            format!("{prefix}/{name}")
        };
        validate_path(&path)?;
        let metadata = dir.symlink_metadata(&name).map_err(err)?;
        if metadata.is_dir() {
            walk(
                &dir.open_dir(&name).map_err(err)?,
                &path,
                files,
                total,
                entries,
            )?;
        } else if metadata.is_file() {
            if files.len() > MAX_FILES {
                return Err("skill exceeds 256 files".into());
            }
            let bytes = read_bytes(dir, &name)?;
            if path == MANIFEST {
                if bytes.len() > 8 * 1024 {
                    return Err("skill manifest exceeds 8 KiB".into());
                }
            } else {
                *total += bytes.len();
            }
            if *total > MAX_PACKAGE_BYTES {
                return Err("skill package exceeds 8 MiB".into());
            }
            files.insert(path, bytes);
        } else {
            return Err("skill files cannot be symlinks or special files".into());
        }
    }
    Ok(())
}
pub(super) fn read_bytes(dir: &Dir, path: &str) -> Result<Vec<u8>, String> {
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use cap_std::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NONBLOCK | libc::O_NOFOLLOW);
    }
    let f = dir.open_with(path, &options).map_err(err)?;
    if !f.metadata().map_err(err)?.is_file() {
        return Err("skill resource is not a regular file".into());
    }
    let mut bytes = vec![];
    f.take((MAX_FILE_BYTES + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(err)?;
    if bytes.len() > MAX_FILE_BYTES {
        return Err("skill file exceeds 256 KiB".into());
    }
    Ok(bytes)
}
fn write_new(dir: &Dir, path: &str, bytes: &[u8]) -> Result<(), String> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use cap_std::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = dir.open_with(path, &options).map_err(err)?;
    file.write_all(bytes).map_err(err)?;
    file.sync_all().map_err(err)
}
fn private_dir(dir: &Dir, path: &str) -> Result<(), String> {
    let mut builder = DirBuilder::new();
    #[cfg(unix)]
    {
        use cap_std::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    dir.create_dir_with(path, &builder).map_err(err)
}
fn create_subdirs(dir: &Dir, path: &Path) -> Result<(), String> {
    let mut current = dir.try_clone().map_err(err)?;
    for component in path.components() {
        let Component::Normal(name) = component else {
            return Err("unsafe skill directory".into());
        };
        match current.symlink_metadata(name) {
            Ok(metadata) if metadata.is_dir() => {}
            Ok(_) => return Err("skill directory is a symlink or file".into()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                private_dir(&current, name.to_str().ok_or("non-UTF-8 directory")?)?
            }
            Err(error) => return Err(err(error)),
        }
        current = current.open_dir(name).map_err(err)?;
    }
    Ok(())
}
pub(super) fn open_root(path: &Path, create: bool) -> Result<Dir, String> {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir().map_err(err)?.join(path)
    };
    let mut components = absolute.components();
    let first = components.next().ok_or("invalid skill root")?;
    #[cfg(unix)]
    let mut current = Dir::open_ambient_dir("/", ambient_authority()).map_err(err)?;
    #[cfg(windows)]
    let mut current = {
        let mut anchor = PathBuf::from(first.as_os_str());
        if matches!(components.clone().next(), Some(Component::RootDir)) {
            anchor.push("\\");
            components.next();
        }
        Dir::open_ambient_dir(anchor, ambient_authority()).map_err(err)?
    };
    #[cfg(unix)]
    if !matches!(first, Component::RootDir) {
        return Err("skill root must be absolute".into());
    }
    for component in components {
        match component {
            Component::RootDir => continue,
            Component::Normal(name) => {
                match current.symlink_metadata(name) {
                    Ok(metadata) if metadata.is_dir() => {}
                    Ok(_) => return Err("skill root contains a symlink or non-directory".into()),
                    Err(error) if create && error.kind() == std::io::ErrorKind::NotFound => {
                        private_dir(&current, name.to_str().ok_or("non-UTF-8 skill root")?)?
                    }
                    Err(error) => return Err(err(error)),
                }
                current = current.open_dir(name).map_err(err)?;
            }
            _ => return Err("unsafe skill root".into()),
        }
    }
    Ok(current)
}
fn lock(dir: &Dir) -> Result<std::fs::File, String> {
    let mut options = OpenOptions::new();
    options.read(true).write(true).create(true);
    #[cfg(unix)]
    {
        use cap_std::fs::OpenOptionsExt;
        options
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    let file = dir
        .open_with(".skills.lock", &options)
        .map_err(err)?
        .into_std();
    if !file.metadata().map_err(err)?.is_file() {
        return Err("skill lock is not a file".into());
    }
    file.lock_exclusive().map_err(err)?;
    Ok(file)
}

// Updates hold the same root lease as readers. If publication was interrupted
// between directory renames, restore the fully validated previous generation.
fn recover(root: &Dir, scope: SkillScope) -> Vec<String> {
    let mut issues = vec![];
    let result = (|| {
        for entry in root.entries().map_err(err)?.take(1024) {
            let entry = entry.map_err(err)?;
            let name = entry.file_name();
            let Some(name) = name.to_str() else {
                continue;
            };
            let Some(suffix) = name.strip_prefix(".previous-") else {
                continue;
            };
            if uuid::Uuid::parse_str(suffix).is_err() {
                continue;
            }
            let restore = (|| {
                if !root.symlink_metadata(name).map_err(err)?.is_dir() {
                    return Err("previous skill generation is not a directory".into());
                }
                let dir = root.open_dir(name).map_err(err)?;
                let bytes = read_bytes(&dir, "SKILL.md")?;
                let metadata = parse_skill(
                    std::str::from_utf8(&bytes).map_err(|_| "SKILL.md must be UTF-8")?,
                )?;
                load_directory(&dir, &metadata.name, scope)?;
                match root.symlink_metadata(&metadata.name) {
                    Ok(_) => {
                        load(root, &metadata.name, scope)?;
                        root.remove_dir_all(name).map_err(err)?;
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                        root.rename(name, root, &metadata.name).map_err(err)?
                    }
                    Err(error) => return Err(err(error)),
                }
                Ok::<_, String>(())
            })();
            if let Err(error) = restore {
                if issues.len() < 64 {
                    issues.push(format!("Skill update recovery: {error}"));
                }
            }
        }
        Ok::<_, String>(())
    })();
    if let Err(error) = result {
        issues.push(error);
    }
    issues
}

#[cfg(test)]
mod local_tests {
    use super::*;
    fn package(description: &str) -> SkillDraftInput {
        SkillDraftInput {
            name: "demo".into(),
            title: None,
            description: description.into(),
            instructions: "Instructions".into(),
            files: None,
        }
    }
    #[test]
    fn canonical_cli_and_atlas_fallbacks_preserve_precedence_and_authority() {
        let t = tempfile::tempdir().unwrap();
        let workspace = t.path().join("workspace");
        let cli = SkillStore::new(t.path().join("cli"), workspace.clone());
        let atlas = SkillStore::new(t.path().join("atlas"), workspace.clone());
        atlas
            .create(package("Atlas fallback"), SkillScope::Personal)
            .unwrap();
        let mut store = SkillStore::new(t.path().join("primary"), workspace.clone());
        store.fallbacks = vec![t.path().join("cli"), t.path().join("atlas")];
        let old = store.read("personal:demo", None).unwrap();
        cli.create(package("Atlas fallback"), SkillScope::Personal)
            .unwrap();
        assert!(store
            .update_document("personal:demo", &old.content, &old.fingerprint)
            .is_err());
        assert!(store
            .create(package("Duplicate"), SkillScope::Personal)
            .is_err());
        assert_eq!(store.list().unwrap().skills.len(), 1);
        store.set_enabled("personal:demo", false).unwrap();
        assert!(!cli.read("personal:demo", None).unwrap().skill.enabled);
        assert!(atlas.read("personal:demo", None).unwrap().skill.enabled);
        let doc = store.read("personal:demo", None).unwrap();
        store
            .update_document(
                "personal:demo",
                &doc.content.replace("Atlas fallback", "CLI edited"),
                &doc.fingerprint,
            )
            .unwrap();
        assert_eq!(
            cli.read("personal:demo", None).unwrap().skill.description,
            "CLI edited"
        );
        assert_eq!(
            atlas.read("personal:demo", None).unwrap().skill.description,
            "Atlas fallback"
        );
        store.set_enabled("personal:demo", true).unwrap();
        assert_eq!(
            store.snapshot().unwrap()[0].directory,
            t.path().join("cli/demo")
        );
        let primary = SkillStore::new(t.path().join("primary"), workspace);
        primary
            .create(package("Primary"), SkillScope::Personal)
            .unwrap();
        assert_eq!(store.list().unwrap().skills.len(), 1);
        assert_eq!(
            store.read("personal:demo", None).unwrap().skill.description,
            "Primary"
        );
    }
    #[test]
    fn editor_rejects_changed_personal_root_even_with_identical_content() {
        let t = tempfile::tempdir().unwrap();
        let legacy = t.path().join("legacy");
        let old = SkillStore::new(legacy.clone(), t.path().join("workspace"));
        old.create(package("Same bytes"), SkillScope::Personal)
            .unwrap();
        let mut current = SkillStore::new(t.path().join("shared"), t.path().join("workspace"));
        current.fallbacks = vec![legacy];
        let document = current.read("personal:demo", None).unwrap();
        let primary = SkillStore::new(t.path().join("shared"), t.path().join("workspace"));
        primary
            .create(package("Same bytes"), SkillScope::Personal)
            .unwrap();
        assert!(current
            .update_document(
                "personal:demo",
                &document.content.replace("Instructions", "Changed"),
                &document.fingerprint
            )
            .is_err());
        assert_eq!(
            primary.read("personal:demo", None).unwrap().content,
            document.content
        );
        assert_eq!(
            old.read("personal:demo", None).unwrap().content,
            document.content
        );
    }
    #[test]
    fn fallback_names_keep_authority_and_primary_wins_without_moving() {
        let t = tempfile::tempdir().unwrap();
        let legacy = t.path().join("legacy");
        let old = SkillStore::new(legacy.clone(), t.path().join("workspace"));
        old.create(package("Legacy"), SkillScope::Personal).unwrap();
        let mut current = SkillStore::new(t.path().join("shared"), t.path().join("workspace"));
        current.fallbacks = vec![legacy.clone()];
        assert_eq!(current.list().unwrap().skills[0].description, "Legacy");
        assert!(current
            .create(package("duplicate"), SkillScope::Personal)
            .is_err());
        current.set_enabled("personal:demo", false).unwrap();
        let doc = current.read("personal:demo", None).unwrap();
        current
            .update_document(
                "personal:demo",
                &doc.content.replace("Legacy", "Edited"),
                &doc.fingerprint,
            )
            .unwrap();
        assert_eq!(
            old.read("personal:demo", None).unwrap().skill.description,
            "Edited"
        );
        assert!(!old.read("personal:demo", None).unwrap().skill.enabled);
        let primary = SkillStore::new(t.path().join("shared"), t.path().join("workspace"));
        primary
            .create(package("Primary"), SkillScope::Personal)
            .unwrap();
        assert_eq!(current.list().unwrap().skills.len(), 1);
        assert_eq!(
            current
                .read("personal:demo", None)
                .unwrap()
                .skill
                .description,
            "Primary"
        );
        assert!(legacy.join("demo/SKILL.md").exists());
    }
}
