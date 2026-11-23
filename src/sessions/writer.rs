use super::{io, ContextCursor, SessionEvent, SessionMetadata};
use crate::{conversations::ConversationId, observability::SessionConfig};
use chrono::{DateTime, FixedOffset};
use fs2::FileExt;
use serde_json::{json, Value};
use std::{
    fs::{self, File},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};
struct State {
    metadata: SessionMetadata,
    error: Option<String>,
}
#[derive(Clone)]
pub struct SessionJournal {
    directory: PathBuf,
    config: SessionConfig,
    state: Arc<Mutex<State>>,
    _lease: Arc<File>,
}
impl SessionJournal {
    pub(super) fn open(
        directory: PathBuf,
        id: &ConversationId,
        first: DateTime<FixedOffset>,
        config: SessionConfig,
    ) -> Result<Self, String> {
        let lease = io::options()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(directory.join("session.lock"))
            .map_err(|_| "Could not open session lease")?;
        lease
            .try_lock_exclusive()
            .map_err(|_| "Session is already running")?;
        io::directory(&directory, Path::new("events"))?;
        let path = directory.join("session.json");
        let mut metadata: SessionMetadata = if path.exists() {
            io::read(&path, config.max_event_bytes)?
        } else {
            SessionMetadata {
                version: 1,
                id: id.clone(),
                first_sent_at: first,
                last_sequence: 0,
                active_turn: None,
                last_finished_turn: None,
                context_cursor: Default::default(),
            }
        };
        if metadata.version != 1 || metadata.id != *id || metadata.first_sent_at != first {
            return Err("Session metadata does not match its identity".into());
        }
        let events = read_events(&directory, &config, id)?;
        metadata.last_sequence = 0;
        metadata.active_turn = None;
        metadata.last_finished_turn = None;
        metadata.context_cursor = Default::default();
        for event in &events {
            apply(&mut metadata, event)?;
        }
        io::write(&path, &metadata, config.sync_each_event)?;
        let journal = Self {
            directory,
            config,
            state: Arc::new(Mutex::new(State {
                metadata,
                error: None,
            })),
            _lease: Arc::new(lease),
        };
        if let Some(turn) = journal.metadata()?.active_turn {
            journal.record(&turn,"turn_recovered",json!({"status":"interrupted","reason":"Previous writer exited before completing this turn"}))?;
        }
        Ok(journal)
    }
    pub fn id(&self) -> ConversationId {
        self.state
            .lock()
            .expect("session state")
            .metadata
            .id
            .clone()
    }
    pub fn directory(&self) -> &Path {
        &self.directory
    }
    pub fn metadata(&self) -> Result<SessionMetadata, String> {
        Ok(self
            .state
            .lock()
            .map_err(|_| "Session state unavailable")?
            .metadata
            .clone())
    }
    pub fn check(&self) -> Result<(), String> {
        match &self
            .state
            .lock()
            .map_err(|_| "Session state unavailable")?
            .error
        {
            Some(error) => Err(error.clone()),
            None => Ok(()),
        }
    }
    pub fn start_turn(&self, input: &str) -> Result<String, String> {
        if self.metadata()?.active_turn.is_some() {
            return Err("Session already has an active turn".into());
        }
        let turn = uuid::Uuid::new_v4().to_string();
        self.record(&turn, "turn_started", json!({"input":input}))?;
        Ok(turn)
    }
    pub fn finish_turn(&self, turn: &str, status: &str) -> Result<(), String> {
        if !["succeeded", "failed", "stopped"].contains(&status) {
            return Err("Invalid session turn status".into());
        }
        self.record(turn, "turn_finished", json!({"status":status}))?;
        Ok(())
    }
    pub fn prepare_context(
        &self,
        turn: &str,
        mut cursor: ContextCursor,
        data: Value,
    ) -> Result<(), String> {
        cursor.prepared_event = None;
        self.record(
            turn,
            "context_prepared",
            json!({"cursor":cursor,"selection":data}),
        )?;
        Ok(())
    }
    pub fn record(&self, turn: &str, kind: &str, mut data: Value) -> Result<u64, String> {
        let mut state = self.state.lock().map_err(|_| "Session state unavailable")?;
        if let Some(error) = &state.error {
            return Err(error.clone());
        }
        let result: Result<u64, String> = (|| {
            io::directory(&self.directory, Path::new("events"))?;
            crate::diagnostics::redact_artifact(&mut data);
            redact_embedded(&mut data, 0);
            let truncated = bound(&mut data, self.config.max_text_chars);
            let sequence = state
                .metadata
                .last_sequence
                .checked_add(1)
                .ok_or("Session sequence exhausted")?;
            let event = SessionEvent {
                version: 1,
                session_id: state.metadata.id.clone(),
                turn_id: turn.into(),
                sequence,
                timestamp: chrono::Utc::now(),
                kind: kind.into(),
                data,
                truncated,
            };
            let bytes =
                serde_json::to_vec_pretty(&event).map_err(|_| "Could not encode session event")?;
            if bytes.len().saturating_add(1) > self.config.max_event_bytes {
                return Err("Session event exceeds configured byte limit".into());
            }
            let path = self
                .directory
                .join("events")
                .join(format!("{sequence:020}.json"));
            if path.exists() {
                return Err("Session event sequence already exists".into());
            }
            let mut candidate = state.metadata.clone();
            apply(&mut candidate, &event)?;
            io::write(&path, &event, self.config.sync_each_event)?;
            io::write(
                &self.directory.join("session.json"),
                &candidate,
                self.config.sync_each_event,
            )?;
            state.metadata = candidate;
            let _trace = crate::diagnostics::session_trace(&event.session_id.to_string(), turn);
            crate::diagnostics::event(
                "DEBUG",
                "session.event",
                json!({"sessionId":event.session_id,"turnId":turn,"sequence":sequence,"kind":kind,"truncated":truncated,"callId":event.data.get("callId"),"model":event.data.get("model"),"tool":event.data.get("tool"),"agentRunId":event.data.get("agentRunId"),"usage":event.data.get("usage")}),
            );
            Ok(sequence)
        })();
        if let Err(error) = &result {
            state.error = Some(error.clone());
        }
        result
    }
    pub fn events(&self) -> Result<Vec<SessionEvent>, String> {
        read_events(&self.directory, &self.config, &self.id())
    }
}
fn apply(metadata: &mut SessionMetadata, event: &SessionEvent) -> Result<(), String> {
    if event.version != 1
        || event.session_id != metadata.id
        || event.sequence != metadata.last_sequence + 1
    {
        return Err("Session event sequence or identity is invalid".into());
    }
    let late_child_completion = event.kind == "child_run_finished"
        && metadata.active_turn.is_none()
        && metadata.last_finished_turn.as_deref() == Some(&event.turn_id);
    if event.kind != "turn_started"
        && !late_child_completion
        && metadata.active_turn.as_deref() != Some(&event.turn_id)
    {
        return Err("Session event belongs to another turn".into());
    }
    match event.kind.as_str() {
        "turn_started" => {
            if metadata.active_turn.is_some() {
                return Err("Overlapping session turns".into());
            }
            metadata.active_turn = Some(event.turn_id.clone());
        }
        "turn_finished" | "turn_recovered" => {
            if metadata.active_turn.as_deref() != Some(&event.turn_id) {
                return Err("Session turn identity is invalid".into());
            }
            metadata.active_turn = None;
            metadata.last_finished_turn = Some(event.turn_id.clone());
        }
        "context_prepared" => {
            let mut cursor: ContextCursor = serde_json::from_value(event.data["cursor"].clone())
                .map_err(|_| "Invalid session context cursor")?;
            cursor.prepared_event = Some(event.sequence);
            metadata.context_cursor = cursor;
        }
        _ => {}
    }
    metadata.last_sequence = event.sequence;
    Ok(())
}
fn read_events(
    directory: &Path,
    config: &SessionConfig,
    id: &ConversationId,
) -> Result<Vec<SessionEvent>, String> {
    let events_dir = io::directory(directory, Path::new("events"))?;
    let mut paths = fs::read_dir(events_dir)
        .map_err(|_| "Could not list session events")?
        .map(|e| e.map(|e| e.path()))
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| "Could not list session events")?;
    paths.retain(|p| p.extension().is_some_and(|e| e == "json"));
    paths.sort();
    let mut events = Vec::new();
    for (index, path) in paths.into_iter().enumerate() {
        let sequence = index as u64 + 1;
        if path.file_stem().and_then(|p| p.to_str()) != Some(&format!("{sequence:020}")) {
            return Err("Session event files have missing or invalid sequences".into());
        }
        let event: SessionEvent = io::read(&path, config.max_event_bytes)?;
        if event.version != 1 || event.session_id != *id || event.sequence != sequence {
            return Err("Session event identity is invalid".into());
        }
        events.push(event);
    }
    Ok(events)
}
fn bound(value: &mut Value, limit: usize) -> bool {
    match value {
        Value::String(text) if text.chars().count() > limit => {
            *text = text.chars().take(limit).collect();
            true
        }
        Value::Object(fields) => {
            let mut truncated = false;
            for (key, value) in fields {
                if [
                    "apikey",
                    "api_key",
                    "authorization",
                    "password",
                    "secret",
                    "token",
                    "access_token",
                    "refresh_token",
                ]
                .contains(&key.to_ascii_lowercase().as_str())
                {
                    *value = json!("[redacted]");
                } else {
                    truncated |= bound(value, limit);
                }
            }
            truncated
        }
        Value::Array(values) => {
            let mut truncated = false;
            for value in values {
                truncated |= bound(value, limit);
            }
            truncated
        }
        _ => false,
    }
}

fn redact_embedded(value: &mut Value, depth: usize) -> bool {
    match value {
        Value::String(text)
            if depth < 16
                && (text.trim_start().starts_with('{') || text.trim_start().starts_with('[')) =>
        {
            if let Ok(mut parsed) = serde_json::from_str::<Value>(text) {
                if redact_embedded(&mut parsed, depth + 1) {
                    if let Ok(redacted) = serde_json::to_string(&parsed) {
                        *text = redacted;
                        return true;
                    }
                }
            }
            false
        }
        Value::Object(fields) => {
            let mut changed = false;
            for (key, value) in fields {
                if [
                    "apikey",
                    "api_key",
                    "authorization",
                    "password",
                    "secret",
                    "token",
                    "access_token",
                    "refresh_token",
                ]
                .contains(&key.to_ascii_lowercase().as_str())
                {
                    *value = json!("[redacted]");
                    changed = true;
                } else {
                    changed |= redact_embedded(value, depth);
                }
            }
            changed
        }
        Value::Array(values) => {
            let mut changed = false;
            for value in values {
                changed |= redact_embedded(value, depth);
            }
            changed
        }
        _ => false,
    }
}
