use super::{io, writer::SessionJournal};
use crate::{conversations::ConversationId, observability::SessionConfig};
use chrono::{DateTime, FixedOffset};
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};
#[derive(Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Index {
    version: u32,
    sessions: BTreeMap<String, DateTime<FixedOffset>>,
    drafts: BTreeMap<String, String>,
}
#[derive(Clone)]
pub struct SessionStore {
    pub(super) root: PathBuf,
    pub(super) config: SessionConfig,
}
impl SessionStore {
    pub fn new(root: &Path, config: SessionConfig) -> Result<Self, String> {
        config.validate()?;
        Ok(Self {
            root: root.to_owned(),
            config,
        })
    }
    pub fn open(
        &self,
        id: Option<&ConversationId>,
        draft: Option<&str>,
        first_sent_at: DateTime<FixedOffset>,
    ) -> Result<SessionJournal, String> {
        let root = io::directory(&self.root, Path::new(&self.config.directory))?;
        let _lock = index_lock(&root)?;
        let path = root.join("index.json");
        let mut index = if path.exists() {
            let index: Index = io::read(&path, self.config.max_index_bytes)?;
            if index.version != 1 {
                return Err("Unsupported session index version".into());
            }
            index
        } else {
            Index {
                version: 1,
                ..Default::default()
            }
        };
        let draft = draft.map(|key| format!("{:x}", Sha256::digest(key.as_bytes())));
        let draft_id = draft
            .as_ref()
            .and_then(|key| index.drafts.get(key))
            .map(|id| ConversationId::parse(id))
            .transpose()
            .map_err(|_| "Invalid session draft identity")?;
        let id = id.cloned().or(draft_id).unwrap_or_default();
        let first = *index
            .sessions
            .get(&id.to_string())
            .unwrap_or(&first_sent_at);
        let relative = relative_path(first, &id);
        io::directory(&root, relative.parent().ok_or("Missing session directory")?)?;
        let journal = SessionJournal::open(
            root.clone(),
            root.join(relative),
            &id,
            first,
            self.config.clone(),
        )?;
        index.sessions.insert(id.to_string(), first);
        if let Some(draft) = draft {
            index.drafts.insert(draft, id.to_string());
        }
        self.publish_index(&path, &index)?;
        Ok(journal)
    }
    fn publish_index(&self, path: &Path, index: &Index) -> Result<(), String> {
        if serde_json::to_vec_pretty(index)
            .map_err(|_| "Could not encode session index")?
            .len()
            .saturating_add(1)
            > self.config.max_index_bytes
        {
            return Err(
                "Session index exceeds configured byte limit; previous index retained".into(),
            );
        }
        io::write(path, index, self.config.sync_each_event)
    }
    pub fn find(&self, id: &ConversationId) -> Result<Option<PathBuf>, String> {
        let root = self.root.join(&self.config.directory);
        let path = root.join("index.json");
        if !path.exists() {
            return Ok(None);
        }
        let index: Index = io::read(&path, self.config.max_index_bytes)?;
        if index.version != 1 {
            return Err("Unsupported session index version".into());
        }
        index
            .sessions
            .get(&id.to_string())
            .map(|first| {
                let path = root.join(relative_path(*first, id));
                match std::fs::symlink_metadata(&path) {
                    Ok(_) => Ok(path),
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                        Ok(path.with_extension(""))
                    }
                    Err(_) => Err("Could not inspect session JSON".into()),
                }
            })
            .transpose()
    }
}

pub struct ArchivedSession {
    store: SessionStore,
    id: ConversationId,
    first: DateTime<FixedOffset>,
    drafts: Vec<String>,
    original: PathBuf,
    archived: PathBuf,
    _lease: std::fs::File,
}
impl ArchivedSession {
    pub fn restore(self) -> Result<(), String> {
        let root = io::directory(&self.store.root, Path::new(&self.store.config.directory))?;
        let _lock = index_lock(&root)?;
        let path = root.join("index.json");
        let mut index: Index = io::read(&path, self.store.config.max_index_bytes)?;
        if index.version != 1
            || self.original.exists()
            || index.sessions.contains_key(&self.id.to_string())
        {
            return Err("Session archive cannot replace an existing session".into());
        }
        std::fs::rename(&self.archived, &self.original)
            .map_err(|_| "Could not restore archived session")?;
        index.sessions.insert(self.id.to_string(), self.first);
        for key in &self.drafts {
            index.drafts.insert(key.clone(), self.id.to_string());
        }
        if let Err(error) = self.store.publish_index(&path, &index) {
            let _ = std::fs::rename(&self.original, &self.archived);
            return Err(error);
        }
        Ok(())
    }
}
impl SessionStore {
    pub fn archive(&self, id: &ConversationId) -> Result<Option<ArchivedSession>, String> {
        let root = self.root.join(&self.config.directory);
        if !root.join("index.json").exists() {
            return Ok(None);
        }
        let root = io::directory(&self.root, Path::new(&self.config.directory))?;
        let _index_lock = index_lock(&root)?;
        let path = root.join("index.json");
        let mut index: Index = io::read(&path, self.config.max_index_bytes)?;
        if index.version != 1 {
            return Err("Unsupported session index version".into());
        }
        let Some(first) = index.sessions.get(&id.to_string()).copied() else {
            return Ok(None);
        };
        let _lease = session_lease(&root, id)?;
        let file = root.join(relative_path(first, id));
        io::directory(
            &root,
            relative_path(first, id)
                .parent()
                .ok_or("Missing session directory")?,
        )?;
        let original = match std::fs::symlink_metadata(&file) {
            Ok(meta) if meta.is_file() && !meta.file_type().is_symlink() => file.clone(),
            Ok(_) => return Err("Unsafe session JSON file".into()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => file.with_extension(""),
            Err(_) => return Err("Could not inspect session JSON".into()),
        };
        let legacy_lease = super::legacy::lease(&root, &file.with_extension(""))?;
        if original == file && legacy_lease.is_some() {
            return Err(
                "Session conversion cleanup is pending; open the session before archiving".into(),
            );
        }
        if original.is_dir() && legacy_lease.is_none() {
            return Err("Missing legacy session lease".into());
        }
        let trash = io::directory(&self.root, Path::new("trash"))?;
        let archived = trash.join(format!("{id}.session"));
        if archived.exists() {
            return Err("Session archive already exists".into());
        }
        let drafts = index
            .drafts
            .iter()
            .filter(|(_, value)| **value == id.to_string())
            .map(|(key, _)| key.clone())
            .collect::<Vec<_>>();
        std::fs::rename(&original, &archived).map_err(|_| "Could not archive session")?;
        drop(legacy_lease);
        index.sessions.remove(&id.to_string());
        for key in &drafts {
            index.drafts.remove(key);
        }
        if let Err(error) = self.publish_index(&path, &index) {
            let _ = std::fs::rename(&archived, &original);
            return Err(error);
        }
        Ok(Some(ArchivedSession {
            store: self.clone(),
            id: id.clone(),
            first,
            drafts,
            original,
            archived,
            _lease,
        }))
    }
}
fn index_lock(root: &Path) -> Result<std::fs::File, String> {
    let lock = io::options()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(root.join("index.lock"))
        .map_err(|_| "Could not open session index lock")?;
    lock.lock_exclusive()
        .map_err(|_| "Could not lock session index")?;
    Ok(lock)
}

fn relative_path(first: DateTime<FixedOffset>, id: &ConversationId) -> PathBuf {
    PathBuf::from(first.format("%Y/%m/%d").to_string())
        .join(format!("{}_{id}.json", first.format("%H%M%S%6f")))
}

pub(super) fn session_lease(root: &Path, id: &ConversationId) -> Result<std::fs::File, String> {
    let locks = io::directory(root, Path::new(".locks"))?;
    let lease = io::options()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(locks.join(format!("{id}.lock")))
        .map_err(|_| "Could not open session lease")?;
    lease
        .try_lock_exclusive()
        .map_err(|_| "Session is already running")?;
    Ok(lease)
}
