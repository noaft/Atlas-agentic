//! Read-only compatibility with the original directory-based session journal.
use super::{io, SessionDocument, SessionEvent, SessionMetadata};
use crate::{conversations::ConversationId, observability::SessionConfig};
use chrono::{DateTime, FixedOffset};
use fs2::FileExt;
use std::{
    fs::{self, File},
    path::Path,
};

pub(super) fn lease(root: &Path, path: &Path) -> Result<Option<File>, String> {
    match fs::symlink_metadata(path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err("Could not inspect legacy session journal".into()),
        Ok(meta) if !meta.is_dir() || meta.file_type().is_symlink() => {
            return Err("Unsafe legacy session directory".into());
        }
        Ok(_) => {}
    }
    io::directory(
        root,
        path.strip_prefix(root)
            .map_err(|_| "Invalid session storage path")?,
    )?;
    let lease = io::options()
        .read(true)
        .write(true)
        .open(path.join("session.lock"))
        .map_err(|_| "Could not open legacy session lease")?;
    lease
        .try_lock_exclusive()
        .map_err(|_| "Session is already running")?;
    Ok(Some(lease))
}

pub(super) fn read(
    directory: &Path,
    config: &SessionConfig,
    id: &ConversationId,
    first: DateTime<FixedOffset>,
) -> Result<SessionDocument, String> {
    let metadata: SessionMetadata =
        io::read(&directory.join("session.json"), config.max_event_bytes)?;
    if metadata.version != 1 || metadata.id != *id || metadata.first_sent_at != first {
        return Err("Legacy session metadata does not match its identity".into());
    }
    let events_dir = directory.join("events");
    let meta =
        fs::symlink_metadata(&events_dir).map_err(|_| "Could not inspect legacy session events")?;
    if !meta.is_dir() || meta.file_type().is_symlink() {
        return Err("Unsafe legacy session events directory".into());
    }
    let mut paths = fs::read_dir(events_dir)
        .map_err(|_| "Could not list session events")?
        .map(|entry| entry.map(|entry| entry.path()))
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| "Could not list session events")?;
    paths.retain(|path| path.extension().is_some_and(|ext| ext == "json"));
    paths.sort();
    let mut events = Vec::new();
    let mut total = 0usize;
    for (index, path) in paths.into_iter().enumerate() {
        let sequence = index as u64 + 1;
        if path.file_stem().and_then(|p| p.to_str()) != Some(&format!("{sequence:020}")) {
            return Err("Session event files have missing or invalid sequences".into());
        }
        let event: SessionEvent = io::read(&path, config.max_event_bytes)?;
        total = total.saturating_add(
            serde_json::to_vec_pretty(&event)
                .map_err(|_| "Could not encode session event")?
                .len(),
        );
        if total > config.max_session_bytes {
            return Err("Session JSON exceeds configured size limit".into());
        }
        events.push(event);
    }
    Ok(SessionDocument {
        version: 2,
        metadata,
        events,
    })
}
