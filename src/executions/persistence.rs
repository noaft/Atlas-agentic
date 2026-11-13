//! Private bounded execution snapshots. Persist no resource handles or image bytes.
use super::*;
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
};
#[derive(Debug, Clone, Copy, Deserialize)]
pub struct PersistenceLimits {
    pub max_records: usize,
    pub max_bytes: u64,
}
impl Default for PersistenceLimits {
    fn default() -> Self {
        #[derive(Deserialize)]
        struct Config {
            persistence_max_records: usize,
            persistence_max_bytes: u64,
        }
        let config: Config = toml::from_str(include_str!("../../config/execution-defaults.toml"))
            .expect("validated execution storage defaults");
        Self {
            max_records: config.persistence_max_records,
            max_bytes: config.persistence_max_bytes,
        }
    }
}
impl PersistenceLimits {
    fn validate(self) -> Result<(), String> {
        if !(1..=RECORDS_TOTAL).contains(&self.max_records)
            || !(4096..=64 * 1024 * 1024).contains(&self.max_bytes)
        {
            return Err(
                "Execution storage limits require 1–128 records and 4096–67108864 bytes".into(),
            );
        }
        Ok(())
    }
}
pub(super) struct Persistence {
    pub(super) directory: PathBuf,
    root: PathBuf,
    claimed: Mutex<Vec<(PathBuf, File)>>,
    _lease: File,
    pub limits: PersistenceLimits,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Snapshot {
    version: u32,
    records: Vec<SavedRecord>,
}
#[derive(Serialize, Deserialize)]
struct SavedRecord {
    id: String,
    owner: String,
    tool: String,
    status: String,
    revision: u64,
    output: output::Output,
    metadata: serde_json::Value,
    error: Option<String>,
    cancel_requested: bool,
    structured: bool,
    wake_up: protocol::ExecutionPolicy,
    acknowledged: bool,
    request_id: String,
    started_at: String,
    completed_at: Option<String>,
    elapsed_ms: u64,
}
fn options() -> OpenOptions {
    let mut o = OpenOptions::new();
    o.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        o.mode(0o600);
    }
    o
}
fn safe_file(path: &Path) -> Result<(), String> {
    match fs::symlink_metadata(path) {
        Ok(m) if !m.file_type().is_file() => {
            Err("Execution storage rejects symlink or special files".into())
        }
        Ok(_) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(_) => Err("Cannot inspect execution storage".into()),
    }
}
fn private_directory(path: &Path) -> Result<(), String> {
    match fs::symlink_metadata(path) {
        Ok(m) if !m.file_type().is_dir() => {
            return Err("Execution storage must be a directory".into())
        }
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => {
            return Err("Cannot inspect execution storage".into())
        }
        _ => {}
    }
    fs::create_dir_all(path).map_err(|_| "Cannot create execution storage")?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))
            .map_err(|_| "Cannot protect execution storage")?;
    }
    Ok(())
}
fn lease(path: &Path, wait: bool) -> Result<File, String> {
    safe_file(path)?;
    let mut options = OpenOptions::new();
    options.read(true).write(true).create(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let file = options
        .open(path)
        .map_err(|_| "Cannot open execution storage lease")?;
    let end = Instant::now() + Duration::from_millis(500);
    loop {
        match file.try_lock_exclusive() {
            Ok(()) => return Ok(file),
            Err(e)
                if e.kind() == std::io::ErrorKind::WouldBlock && wait && Instant::now() < end =>
            {
                thread::sleep(Duration::from_millis(5))
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                return Err("Execution storage lease is busy".into())
            }
            Err(_) => return Err("Execution storage lease is unavailable".into()),
        }
    }
}
fn journals(root: &Path) -> Result<Vec<PathBuf>, String> {
    let mut directories = vec![];
    for entry in fs::read_dir(root).map_err(|_| "Cannot inspect execution journals")? {
        let entry = entry.map_err(|_| "Cannot inspect execution journals")?;
        if uuid::Uuid::parse_str(&entry.file_name().to_string_lossy()).is_err() {
            continue;
        }
        if !entry
            .file_type()
            .map_err(|_| "Cannot inspect execution journal")?
            .is_dir()
        {
            return Err("Invalid execution journal directory".into());
        }
        directories.push(entry.path());
        if directories.len() > 128 {
            return Err("Execution journal quota exceeded".into());
        }
    }
    directories.sort();
    Ok(directories)
}
impl Persistence {
    pub fn open(root: &Path, limits: PersistenceLimits) -> Result<Self, String> {
        limits.validate()?;
        private_directory(root)?;
        let _import = lease(&root.join("import.lock"), true)?;
        let existing = journals(root)?;
        if existing.len() >= 128 {
            return Err("Execution journal quota exceeded".into());
        }
        let directory = root.join(uuid::Uuid::new_v4().to_string());
        private_directory(&directory)?;
        let own_lease = lease(&directory.join("executions.lock"), false)?;
        Ok(Self {
            directory,
            root: root.into(),
            claimed: Mutex::new(vec![]),
            _lease: own_lease,
            limits,
        })
    }
    pub(super) fn collect_archives(&self) -> Result<Vec<Snapshot>, String> {
        let _import = lease(&self.root.join("import.lock"), true)?;
        let already = self
            .claimed
            .lock()
            .map_err(|_| "Execution storage unavailable")?
            .iter()
            .map(|(path, _)| path.clone())
            .collect::<HashSet<_>>();
        let mut collected = vec![];
        let mut snapshots = vec![];
        let mut remaining_bytes = self.limits.max_bytes;
        for directory in journals(&self.root)? {
            if directory == self.directory || already.contains(&directory) {
                continue;
            }
            let owned = match lease(&directory.join("executions.lock"), false) {
                Ok(owned) => owned,
                Err(error) if error == "Execution storage lease is busy" => continue,
                Err(error) => return Err(error),
            };
            if let Some((snapshot, bytes)) = self.read_snapshot(&directory, remaining_bytes)? {
                remaining_bytes -= bytes;
                snapshots.push(snapshot);
            }
            collected.push((directory, owned));
        }
        self.claimed
            .lock()
            .map_err(|_| "Execution storage unavailable")?
            .extend(collected);
        Ok(snapshots)
    }
    fn read_snapshot(
        &self,
        directory: &Path,
        remaining_bytes: u64,
    ) -> Result<Option<(Snapshot, u64)>, String> {
        let path = directory.join("executions.json");
        safe_file(&path)?;
        let file = match File::open(path) {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(_) => return Err("Cannot read execution storage".into()),
        };
        let mut bytes = vec![];
        file.take(remaining_bytes + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| "Cannot read execution storage")?;
        if bytes.len() as u64 > remaining_bytes {
            return Err("Execution archive collection byte quota exceeded".into());
        }
        let snapshot: Snapshot =
            serde_json::from_slice(&bytes).map_err(|_| "Invalid execution storage")?;
        if snapshot.version != 1 || snapshot.records.len() > self.limits.max_records {
            return Err("Unsupported execution storage version or quota".into());
        }
        Ok(Some((snapshot, bytes.len() as u64)))
    }
    pub(super) fn merge(&self, snapshots: Vec<Snapshot>, s: &mut Store) -> Result<(), String> {
        let mut staged = stage_store(s);
        if let Err(error) = self.merge_staged(snapshots, &mut staged) {
            self.claimed
                .lock()
                .map_err(|_| "Execution storage unavailable")?
                .clear();
            return Err(error);
        }
        for (id, record) in &mut staged.records {
            if let Some(original) = s.records.get_mut(id) {
                record.worker = original.worker.take();
                record.attachments = std::mem::take(&mut original.attachments);
            }
        }
        s.records = staged.records;
        s.mailboxes = staged.mailboxes;
        s.archives = staged.archives;
        s.sequence = staged.sequence;
        Ok(())
    }
    fn merge_staged(&self, snapshots: Vec<Snapshot>, s: &mut Store) -> Result<(), String> {
        for snapshot in snapshots {
            for r in snapshot.records {
                if r.id.len() > 128
                    || r.owner.len() > 512
                    || r.tool.len() > 128
                    || r.error.as_ref().is_some_and(|e| e.len() > 4096)
                    || r.metadata.to_string().len() > 4096
                {
                    return Err("Invalid execution storage record".into());
                }
                if uuid::Uuid::parse_str(&r.request_id).is_err()
                    || chrono::DateTime::parse_from_rfc3339(&r.started_at).is_err()
                    || r.completed_at
                        .as_ref()
                        .is_some_and(|at| chrono::DateTime::parse_from_rfc3339(at).is_err())
                {
                    return Err("Invalid execution storage identity or timestamps".into());
                }
                r.wake_up.interval_ms()?;
                r.output.validate()?;
                let recovered = !terminal(&r.status);
                let status = match r.status.as_str() {
                    "succeeded" => "succeeded",
                    "failed" => "failed",
                    "cancelled" => "cancelled",
                    _ => "interrupted",
                };
                let id = r.id;
                if let Some(existing) = s.records.get(&id) {
                    if !terminal(existing.status) || existing.pending_status.is_some() {
                        return Err(
                            "Cannot replace a live execution with an archived identity".into()
                        );
                    }
                    if alias_owner(s, &existing.owner) != r.owner
                        || existing.request_id != r.request_id
                        || existing.tool != r.tool
                    {
                        return Err("Archived execution identity conflicts with its owner".into());
                    }
                    if existing.revision > r.revision
                        || existing.revision == r.revision
                            && (existing.acknowledged || !r.acknowledged)
                    {
                        continue;
                    }
                }
                let live_owner = s
                    .aliases
                    .iter()
                    .filter_map(|(native, stable)| {
                        if r.owner == *stable {
                            Some((stable.len(), native.clone()))
                        } else {
                            r.owner
                                .strip_prefix(&format!("{stable}/"))
                                .map(|suffix| (stable.len(), format!("{native}/{suffix}")))
                        }
                    })
                    .max_by_key(|(len, _)| *len)
                    .map(|(_, owner)| owner)
                    .unwrap_or(r.owner.clone());
                if !s.records.contains_key(&id)
                    && (s.records.len() >= self.limits.max_records
                        || s.records
                            .values()
                            .filter(|other| other.owner == live_owner)
                            .count()
                            >= RECORDS_OWNER)
                {
                    evict_archived(s, &live_owner, self.limits.max_records)?;
                }
                let notify = recovered || !r.acknowledged;
                s.records.insert(id.clone(), Record {
                owner:live_owner, tool:r.tool,status, started:Instant::now(),request_id:r.request_id,started_at:r.started_at,completed_at:if recovered {Some(chrono::Utc::now().to_rfc3339())} else {r.completed_at},elapsed_offset:r.elapsed_ms,finished_elapsed:Some(r.elapsed_ms),revision:r.revision.saturating_add(u64::from(recovered)), output:r.output, metadata:r.metadata,
                error:if recovered {Some("Execution interrupted by restart; action outcome may be unknown; no action was replayed".into())} else {r.error},
                cancel_requested:r.cancel_requested,cancel_ack:false,structured:r.structured,image_epoch:0,attachments:vec![],worker:None,launched:true,wake_up:r.wake_up,acknowledged:!notify,pending_status:None,restored:true,
            });
                if notify {
                    enqueue(
                        s,
                        &id,
                        if recovered {
                            "interrupted"
                        } else {
                            "recovered_terminal"
                        },
                    );
                }
            }
        }
        Ok(())
    }
}
impl Drop for Persistence {
    fn drop(&mut self) {
        // A tool cannot dispatch before its initial snapshot is committed. An empty
        // runtime journal therefore carries no action whose result must be retained.
        if fs::symlink_metadata(self.directory.join("executions.json")).is_ok() {
            return;
        }
        let Ok(_import) = lease(&self.root.join("import.lock"), true) else {
            return;
        };
        let Ok(mut entries) = fs::read_dir(&self.directory) else {
            return;
        };
        let disposable = entries.all(|entry| {
            entry.is_ok_and(|entry| {
                let name = entry.file_name();
                let name = name.to_string_lossy();
                (name == "executions.lock"
                    || name.starts_with(".execution-") && name.ends_with(".tmp"))
                    && entry.file_type().is_ok_and(|kind| kind.is_file())
            })
        });
        if disposable {
            let _ = FileExt::unlock(&self._lease);
            let _ = fs::remove_dir_all(&self.directory);
        }
    }
}
fn stage_store(s: &Store) -> Store {
    Store {
        records: s
            .records
            .iter()
            .map(|(id, r)| {
                (
                    id.clone(),
                    Record {
                        owner: r.owner.clone(),
                        tool: r.tool.clone(),
                        status: r.status,
                        started: r.started,
                        request_id: r.request_id.clone(),
                        started_at: r.started_at.clone(),
                        completed_at: r.completed_at.clone(),
                        elapsed_offset: r.elapsed_offset,
                        finished_elapsed: r.finished_elapsed,
                        revision: r.revision,
                        output: r.output.clone(),
                        metadata: r.metadata.clone(),
                        error: r.error.clone(),
                        cancel_requested: r.cancel_requested,
                        cancel_ack: r.cancel_ack,
                        structured: r.structured,
                        image_epoch: r.image_epoch,
                        attachments: vec![],
                        worker: None,
                        launched: r.launched,
                        wake_up: r.wake_up.clone(),
                        acknowledged: r.acknowledged,
                        pending_status: r.pending_status,
                        restored: r.restored,
                    },
                )
            })
            .collect(),
        mailboxes: s
            .mailboxes
            .iter()
            .map(|(owner, b)| {
                (
                    owner.clone(),
                    Mailbox {
                        pending: b.pending.clone(),
                        in_flight: b.in_flight.clone(),
                        consumer: b.consumer,
                        image_epoch: b.image_epoch,
                        request_id: b.request_id.clone(),
                    },
                )
            })
            .collect(),
        aliases: s.aliases.clone(),
        closed: s.closed.clone(),
        archives: s.archives.clone(),
        sequence: s.sequence,
        shutdown: s.shutdown,
        ..Store::default()
    }
}
fn evict_archived(s: &mut Store, owner: &str, limit: usize) -> Result<(), String> {
    while s.records.len() >= limit
        || s.records.values().filter(|r| r.owner == owner).count() >= RECORDS_OWNER
    {
        let per_owner = s.records.values().filter(|r| r.owner == owner).count() >= RECORDS_OWNER;
        let old = s
            .records
            .iter()
            .filter(|(_, r)| {
                terminal(r.status) && r.acknowledged && (!per_owner || r.owner == owner)
            })
            .min_by_key(|(_, r)| r.started_at.clone())
            .map(|(id, _)| id.clone())
            .ok_or("Execution archive retention is full; consume pending results first")?;
        s.records.remove(&old);
        s.archives.remove(&old);
    }
    Ok(())
}
pub(super) fn publication_guard<'a>(
    shared: &'a Shared,
    mut state: std::sync::MutexGuard<'a, Store>,
) -> (
    std::sync::MutexGuard<'a, Store>,
    std::sync::MutexGuard<'a, ()>,
) {
    let publication = match shared.publication.try_lock() {
        Ok(lock) => lock,
        Err(std::sync::TryLockError::Poisoned(error)) => error.into_inner(),
        Err(std::sync::TryLockError::WouldBlock) => {
            drop(state);
            let lock = shared
                .publication
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            state = shared
                .store
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            lock
        }
    };
    (state, publication)
}
pub(super) fn refresh<'a>(
    shared: &'a Shared,
    state: std::sync::MutexGuard<'a, Store>,
) -> (std::sync::MutexGuard<'a, Store>, Result<(), String>) {
    let (state, publication) = publication_guard(shared, state);
    let Some(p) = state.persistence.clone() else {
        return (state, Ok(()));
    };
    drop(state);
    let snapshots = p.collect_archives();
    let mut state = shared
        .store
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let result = snapshots.and_then(|snapshots| p.merge(snapshots, &mut state));
    drop(publication);
    if result.is_err() {
        return (state, result);
    }
    save(shared, state)
}
fn alias_owner(s: &Store, owner: &str) -> String {
    s.aliases
        .iter()
        .filter_map(|(native, stable)| {
            if native == owner {
                Some((native.len(), stable.clone()))
            } else {
                owner
                    .strip_prefix(&format!("{native}/"))
                    .map(|suffix| (native.len(), format!("{stable}/{suffix}")))
            }
        })
        .max_by_key(|(len, _)| *len)
        .map(|(_, owner)| owner)
        .unwrap_or_else(|| owner.into())
}
pub(super) fn saved_value(s: &Store, id: &str, r: &Record) -> serde_json::Value {
    let mut payload = serde_json::json!({"output":r.output,"metadata":r.metadata,"error":r.error});
    crate::diagnostics::redact_artifact(&mut payload);
    serde_json::json!({
        "id":id,"owner":alias_owner(s,&r.owner),"tool":r.tool,
        "status":r.pending_status.unwrap_or(r.status),"revision":r.revision,
        "output":payload["output"],"metadata":payload["metadata"],"error":payload["error"],
        "cancel_requested":r.cancel_requested,"structured":r.structured,"wake_up":r.wake_up,
        "acknowledged":r.acknowledged,"request_id":r.request_id,"started_at":r.started_at,
        "completed_at":r.completed_at,"elapsed_ms":record_elapsed(r)
    })
}
fn encoded(s: &Store, owner: Option<&str>) -> Result<Vec<u8>, String> {
    let selected = |candidate: &str| {
        owner.is_none_or(|owner| candidate == owner || candidate.starts_with(&format!("{owner}/")))
    };
    let mut records =
        s.records
            .iter()
            .filter(|(_, r)| {
                selected(&r.owner)
                    && !s.closed.iter().any(|closed| {
                        r.owner == *closed || r.owner.starts_with(&format!("{closed}/"))
                    })
            })
            .map(|(id, r)| (id.clone(), saved_value(s, id, r)))
            .collect::<HashMap<_, _>>();
    for (id, value) in &s.archives {
        let original = value["owner"].as_str().unwrap_or_default();
        let stable = alias_owner(s, original);
        if selected(original) || selected(&stable) {
            let mut value = value.clone();
            value["owner"] = stable.into();
            records.entry(id.clone()).or_insert(value);
        }
    }
    serde_json::to_vec(
        &serde_json::json!({"version":1,"records":records.into_values().collect::<Vec<_>>()}),
    )
    .map_err(|_| "Cannot encode execution storage".into())
}
pub(super) fn trash<'a>(
    shared: &'a Shared,
    state: std::sync::MutexGuard<'a, Store>,
    owner: &str,
) -> (std::sync::MutexGuard<'a, Store>, Result<(), String>) {
    let (state, publication) = publication_guard(shared, state);
    let Some(p) = state.persistence.clone() else {
        return (state, Ok(()));
    };
    let bytes = encoded(&state, Some(owner));
    drop(state);
    let result = bytes.and_then(|bytes| publish_trash(&p, bytes));
    drop(publication);
    (
        shared
            .store
            .lock()
            .unwrap_or_else(|error| error.into_inner()),
        result,
    )
}
fn publish_trash(p: &Persistence, bytes: Vec<u8>) -> Result<(), String> {
    let snapshot: Snapshot =
        serde_json::from_slice(&bytes).map_err(|_| "Cannot inspect execution trash publication")?;
    if snapshot.records.is_empty() {
        return Ok(());
    }
    let mut records = snapshot.records.len();
    if bytes.len() as u64 > p.limits.max_bytes || records > p.limits.max_records {
        return Err("Execution trash quota exceeded".into());
    }
    let _import = lease(&p.root.join("import.lock"), true)?;
    let directory = p.root.join("trash");
    match fs::symlink_metadata(&directory) {
        Ok(m) if !m.is_dir() || m.file_type().is_symlink() => {
            return Err("Invalid execution trash directory".into())
        }
        _ => {}
    }
    fs::create_dir_all(&directory).map_err(|_| "Cannot create execution trash")?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&directory, fs::Permissions::from_mode(0o700))
            .map_err(|_| "Cannot protect execution trash")?;
    }
    let mut total = bytes.len() as u64;
    let mut files = 1;
    for entry in fs::read_dir(&directory).map_err(|_| "Cannot inspect execution trash")? {
        let entry = entry.map_err(|_| "Cannot inspect execution trash")?;
        let m = fs::symlink_metadata(entry.path()).map_err(|_| "Cannot inspect execution trash")?;
        if !m.file_type().is_file() {
            return Err("Invalid execution trash entry".into());
        }
        files += 1;
        if files > p.limits.max_records {
            return Err("Execution trash artifact quota exceeded; remove reviewed trash".into());
        }
        total = total.saturating_add(m.len());
        if total > p.limits.max_bytes {
            return Err("Execution trash byte quota exceeded; remove reviewed trash".into());
        }
        let mut prior = vec![];
        File::open(entry.path())
            .map_err(|_| "Cannot read execution trash")?
            .take(p.limits.max_bytes + 1)
            .read_to_end(&mut prior)
            .map_err(|_| "Cannot read execution trash")?;
        let prior: Snapshot =
            serde_json::from_slice(&prior).map_err(|_| "Invalid execution trash metadata")?;
        if prior.version != 1 {
            return Err("Unsupported execution trash version".into());
        }
        records = records.saturating_add(prior.records.len());
        if records > p.limits.max_records {
            return Err("Execution trash record quota exceeded; remove reviewed trash".into());
        }
    }
    if total > p.limits.max_bytes {
        return Err("Execution trash quota exceeded; remove reviewed trash before deleting more conversations".into());
    }
    let temp = directory.join(format!(".{}.tmp", uuid::Uuid::new_v4()));
    let target = directory.join(format!("{}.json", uuid::Uuid::new_v4()));
    let result = (|| {
        let mut f = options()
            .open(&temp)
            .map_err(|_| "Cannot prepare execution trash")?;
        f.write_all(&bytes)
            .and_then(|_| f.sync_all())
            .map_err(|_| "Cannot write execution trash")?;
        fs::rename(&temp, target).map_err(|_| "Cannot publish execution trash")?;
        #[cfg(unix)]
        File::open(&directory)
            .and_then(|f| f.sync_all())
            .map_err(|_| "Cannot commit execution trash directory")?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(temp);
    }
    result
}
type Committed = Vec<(String, u64, &'static str)>;
pub(super) fn save<'a>(
    shared: &'a Shared,
    state: std::sync::MutexGuard<'a, Store>,
) -> (std::sync::MutexGuard<'a, Store>, Result<(), String>) {
    let (state, result) = save_transaction(shared, state);
    (state, result.map(|_| ()))
}
fn save_transaction<'a>(
    shared: &'a Shared,
    state: std::sync::MutexGuard<'a, Store>,
) -> (std::sync::MutexGuard<'a, Store>, Result<Committed, String>) {
    let (state, publication) = publication_guard(shared, state);
    save_with_guard(shared, state, publication)
}
pub(super) fn save_with_guard<'a>(
    shared: &'a Shared,
    state: std::sync::MutexGuard<'a, Store>,
    publication: std::sync::MutexGuard<'a, ()>,
) -> (std::sync::MutexGuard<'a, Store>, Result<Committed, String>) {
    let included = state
        .records
        .iter()
        .filter_map(|(id, r)| {
            r.pending_status
                .map(|status| (id.clone(), r.revision, status))
        })
        .collect::<Vec<_>>();
    let Some(p) = state.persistence.clone() else {
        return (state, Ok(included));
    };
    let bytes = encoded(&state, None);
    drop(state);
    let result = bytes.and_then(|bytes| publish(&p, bytes));
    drop(publication);
    let state = shared
        .store
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    (state, result.map(|_| included))
}
fn publish(p: &Persistence, bytes: Vec<u8>) -> Result<(), String> {
    let record_count = serde_json::from_slice::<Snapshot>(&bytes)
        .map_err(|_| "Cannot inspect execution publication")?
        .records
        .len();
    let _import = lease(&p.root.join("import.lock"), true)?;
    if bytes.len() as u64 > p.limits.max_bytes {
        return Err("Execution storage quota exceeded".into());
    }
    let claimed = p
        .claimed
        .lock()
        .map_err(|_| "Execution storage unavailable")?;
    let mut total_bytes = bytes.len() as u64;
    let mut total_records = record_count;
    for journal in journals(&p.root)? {
        if journal == p.directory || claimed.iter().any(|(path, _)| path == &journal) {
            continue;
        }
        let file = journal.join("executions.json");
        safe_file(&file)?;
        match File::open(&file) {
            Ok(file) => {
                let mut data = vec![];
                file.take(p.limits.max_bytes + 1)
                    .read_to_end(&mut data)
                    .map_err(|_| "Cannot read execution journal quota")?;
                total_bytes = total_bytes.saturating_add(data.len() as u64);
                if total_bytes > p.limits.max_bytes {
                    return Err("Execution storage byte quota exceeded".into());
                }
                let snapshot: Snapshot = serde_json::from_slice(&data)
                    .map_err(|_| "Invalid execution journal quota metadata")?;
                total_records = total_records.saturating_add(snapshot.records.len());
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => return Err("Cannot inspect execution journal quota".into()),
        }
    }
    if total_records > p.limits.max_records {
        return Err("Execution storage record quota exceeded".into());
    }
    drop(claimed);
    let target = p.directory.join("executions.json");
    safe_file(&target)?;
    let temp = p
        .directory
        .join(format!(".execution-{}.tmp", uuid::Uuid::new_v4()));
    let result = (|| {
        let mut file = options()
            .open(&temp)
            .map_err(|_| "Cannot prepare execution storage")?;
        file.write_all(&bytes)
            .and_then(|_| file.sync_all())
            .map_err(|_| "Cannot write execution storage")?;
        fs::rename(&temp, &target).map_err(|_| "Cannot publish execution storage")?;
        #[cfg(unix)]
        File::open(&p.directory)
            .and_then(|f| f.sync_all())
            .map_err(|_| "Cannot commit execution storage directory")?;
        let mut claimed = p
            .claimed
            .lock()
            .map_err(|_| "Execution storage unavailable")?;
        for (directory, owned) in claimed.drain(..) {
            drop(owned);
            fs::remove_dir_all(directory)
                .map_err(|_| "Cannot retire imported execution journal")?;
        }
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(temp);
    }
    result
}
pub(super) fn commit_publication(state: &mut Store, included: Committed) {
    let mut ids = vec![];
    for (id, revision, status) in included {
        if let Some(record) = state.records.get_mut(&id) {
            if record.revision == revision && record.pending_status == Some(status) {
                record.pending_status = None;
                record.status = status;
                ids.push(id);
            }
        }
    }
    for id in ids {
        enqueue(state, &id, "storage_recovered");
    }
}
pub(super) fn retry<'a>(
    shared: &'a Shared,
    state: std::sync::MutexGuard<'a, Store>,
) -> (std::sync::MutexGuard<'a, Store>, Result<(), String>) {
    if !state.records.values().any(|r| r.pending_status.is_some()) {
        return (state, Ok(()));
    }
    let (mut state, result) = save_transaction(shared, state);
    let included = match result {
        Ok(included) => included,
        Err(error) => return (state, Err(error)),
    };
    commit_publication(&mut state, included);
    (state, Ok(()))
}

#[cfg(test)]
mod archive_merge_tests {
    use super::*;
    #[test]
    fn aliased_archive_import_respects_the_native_owner_retention_limit() {
        let root = tempfile::tempdir().unwrap();
        let p = Persistence::open(root.path(), PersistenceLimits::default()).unwrap();
        let mut state = Store::default();
        state.aliases.insert("native".into(), "saved".into());
        let batch = |owner: &str| Snapshot {
            version: 1,
            records: (0..20)
                .map(|_| {
                    let mut record = saved(owner);
                    record.acknowledged = true;
                    record
                })
                .collect(),
        };
        p.merge(vec![batch("native")], &mut state).unwrap();
        p.merge(vec![batch("saved")], &mut state).unwrap();
        assert_eq!(state.records.len(), RECORDS_OWNER);
        assert!(state
            .records
            .values()
            .all(|record| record.owner == "native"));
    }
    #[test]
    fn archive_collection_bounds_aggregate_decoded_bytes_and_releases_claims() {
        let root = tempfile::tempdir().unwrap();
        let mut first = saved("owner");
        first.output.append(&"a".repeat(2500));
        let mut second = saved("owner");
        second.output.append(&"b".repeat(2500));
        let first = journal(root.path(), first);
        let second = journal(root.path(), second);
        assert!(fs::metadata(first.join("executions.json")).unwrap().len() < 4096);
        assert!(fs::metadata(second.join("executions.json")).unwrap().len() < 4096);
        let p = Persistence::open(
            root.path(),
            PersistenceLimits {
                max_records: 128,
                max_bytes: 4096,
            },
        )
        .unwrap();
        assert!(p.collect_archives().is_err());
        assert!(p.claimed.lock().unwrap().is_empty());
        assert!(first.exists() && second.exists());
    }
    #[test]
    fn aliased_import_does_not_evict_unread_native_owner_records() {
        let root = tempfile::tempdir().unwrap();
        let p = Persistence::open(root.path(), PersistenceLimits::default()).unwrap();
        let mut state = Store::default();
        state.aliases.insert("native".into(), "saved".into());
        p.merge(
            vec![Snapshot {
                version: 1,
                records: (0..RECORDS_OWNER).map(|_| saved("native")).collect(),
            }],
            &mut state,
        )
        .unwrap();
        let ids = state.records.keys().cloned().collect::<HashSet<_>>();
        assert!(p
            .merge(
                vec![Snapshot {
                    version: 1,
                    records: vec![saved("saved")]
                }],
                &mut state
            )
            .is_err());
        assert_eq!(state.records.keys().cloned().collect::<HashSet<_>>(), ids);
    }
    fn saved(owner: &str) -> SavedRecord {
        SavedRecord {
            id: uuid::Uuid::new_v4().to_string(),
            owner: owner.into(),
            tool: "fake".into(),
            status: "succeeded".into(),
            revision: 1,
            output: output::Output::default(),
            metadata: serde_json::json!({}),
            error: None,
            cancel_requested: false,
            structured: false,
            wake_up: protocol::ExecutionPolicy::default(),
            acknowledged: false,
            request_id: uuid::Uuid::new_v4().to_string(),
            started_at: chrono::Utc::now().to_rfc3339(),
            completed_at: Some(chrono::Utc::now().to_rfc3339()),
            elapsed_ms: 1,
        }
    }
    fn journal(root: &Path, record: SavedRecord) -> PathBuf {
        let directory = root.join(uuid::Uuid::new_v4().to_string());
        private_directory(&directory).unwrap();
        fs::write(
            directory.join("executions.json"),
            serde_json::to_vec(&Snapshot {
                version: 1,
                records: vec![record],
            })
            .unwrap(),
        )
        .unwrap();
        directory
    }
    #[test]
    fn trash_does_not_create_empty_artifacts_or_exceed_its_record_quota() {
        let root = tempfile::tempdir().unwrap();
        let p = Persistence::open(
            root.path(),
            PersistenceLimits {
                max_records: 1,
                max_bytes: 4096,
            },
        )
        .unwrap();
        let empty = serde_json::to_vec(&Snapshot {
            version: 1,
            records: vec![],
        })
        .unwrap();
        publish_trash(&p, empty).unwrap();
        assert!(
            !root.path().join("trash").exists(),
            "empty chat deletion must not accumulate empty artifacts"
        );
        let bytes = serde_json::to_vec(&Snapshot {
            version: 1,
            records: vec![saved("owner")],
        })
        .unwrap();
        publish_trash(&p, bytes.clone()).unwrap();
        assert!(
            publish_trash(&p, bytes).is_err(),
            "trash must enforce the configured record quota"
        );
        assert_eq!(fs::read_dir(root.path().join("trash")).unwrap().count(), 1);
    }

    #[test]
    fn uncommitted_empty_journals_do_not_accumulate_after_failed_startup() {
        let root = tempfile::tempdir().unwrap();
        let directory;
        {
            let p = Persistence::open(root.path(), PersistenceLimits::default()).unwrap();
            directory = p.directory.clone();
        }
        assert!(!directory.exists());
    }
    #[test]
    fn quota_failed_archive_merge_keeps_live_state_and_all_source_journals() {
        let root = tempfile::tempdir().unwrap();
        let first = journal(root.path(), saved("owner"));
        let second = journal(root.path(), saved("owner"));
        let p = Persistence::open(
            root.path(),
            PersistenceLimits {
                max_records: 1,
                max_bytes: 1024 * 1024,
            },
        )
        .unwrap();
        let mut state = Store::default();
        for _ in 0..2 {
            let snapshots = p.collect_archives().unwrap();
            assert_eq!(snapshots.len(), 2);
            assert!(p.merge(snapshots, &mut state).is_err());
            assert!(
                state.records.is_empty(),
                "Failed imports must not partially mutate live state"
            );
            assert!(state.mailboxes.is_empty());
            assert!(
                p.claimed.lock().unwrap().is_empty(),
                "Failed imports must release all source leases"
            );
            assert!(first.join("executions.json").exists());
            assert!(second.join("executions.json").exists());
        }
    }
    #[test]
    fn malformed_later_journal_does_not_claim_earlier_sources() {
        let root = tempfile::tempdir().unwrap();
        let first = journal(root.path(), saved("owner"));
        let second = journal(root.path(), saved("owner"));
        let later = if first > second { &first } else { &second };
        fs::write(later.join("executions.json"), "invalid").unwrap();
        let p = Persistence::open(root.path(), PersistenceLimits::default()).unwrap();
        assert!(p.collect_archives().is_err());
        assert!(p.claimed.lock().unwrap().is_empty());
        assert!(first.exists());
        assert!(second.exists());
    }
    #[test]
    fn archived_identity_cannot_replace_a_live_worker_record() {
        let manager = ToolExecutionManager::new();
        let scope = manager.scope("owner");
        let run = scope.reserve("fake").unwrap();
        let root = tempfile::tempdir().unwrap();
        let p = Persistence::open(root.path(), PersistenceLimits::default()).unwrap();
        let mut incoming = saved("owner");
        incoming.id = run.execution_id.clone();
        incoming.revision = 99;
        let mut state = manager.0.shared.store.lock().unwrap();
        assert!(p
            .merge(
                vec![Snapshot {
                    version: 1,
                    records: vec![incoming]
                }],
                &mut state
            )
            .is_err());
        assert_eq!(state.records[&run.execution_id].status, "running");
    }
}
