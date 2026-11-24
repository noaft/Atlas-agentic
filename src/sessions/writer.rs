use super::{
    io, ContextCursor, SessionDocument, SessionDocumentRef, SessionEvent, SessionMetadata,
};
use crate::{conversations::ConversationId, observability::SessionConfig};
use chrono::{DateTime, FixedOffset};
use serde_json::{json, Value};
use std::{
    fs::File,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};
struct State {
    metadata: SessionMetadata,
    events: Vec<SessionEvent>,
    error: Option<String>,
}
#[derive(Clone)]
pub struct SessionJournal {
    path: PathBuf,
    root: PathBuf,
    config: SessionConfig,
    state: Arc<Mutex<State>>,
    _lease: Arc<File>,
}
impl SessionJournal {
    pub(super) fn open(
        root: PathBuf,
        path: PathBuf,
        id: &ConversationId,
        first: DateTime<FixedOffset>,
        config: SessionConfig,
    ) -> Result<Self, String> {
        let lease = super::store::session_lease(&root, id)?;
        let legacy_path = path.with_extension("");
        let legacy_lease = super::legacy::lease(&root, &legacy_path)?;
        let legacy_document = legacy_lease
            .as_ref()
            .map(|_| {
                let document = super::legacy::read(&legacy_path, &config, id, first)?;
                validate(&document, &config, id, first)?;
                Ok::<_, String>(document)
            })
            .transpose()?;
        let document = if path
            .try_exists()
            .map_err(|_| "Could not inspect session JSON")?
        {
            io::read::<SessionDocument>(&path, config.max_session_bytes)?
        } else if let Some(document) = legacy_document.as_ref() {
            SessionDocument {
                version: 2,
                metadata: document.metadata.clone(),
                events: document.events.clone(),
            }
        } else {
            SessionDocument {
                version: 2,
                metadata: SessionMetadata {
                    version: 1,
                    id: id.clone(),
                    first_sent_at: first,
                    last_sequence: 0,
                    active_turn: None,
                    last_finished_turn: None,
                    context_cursor: Default::default(),
                },
                events: Vec::new(),
            }
        };
        if let Some(legacy) = legacy_document {
            if !document.events.starts_with(&legacy.events) {
                return Err(
                    "Legacy session differs from its converted JSON; both copies retained".into(),
                );
            }
        }
        let metadata = validate(&document, &config, id, first)?;
        let journal = Self {
            path,
            root,
            config,
            state: Arc::new(Mutex::new(State {
                metadata,
                events: document.events,
                error: None,
            })),
            _lease: Arc::new(lease),
        };
        {
            let state = journal
                .state
                .lock()
                .map_err(|_| "Session state unavailable")?;
            journal.publish(&state.metadata, &state.events)?;
        }
        if legacy_lease.is_some() {
            // Stage the old directory atomically so interruption during recursive
            // cleanup cannot leave a half-deleted source blocking the new file.
            let staging = io::directory(&journal.root, Path::new(".legacy"))?
                .join(format!("{id}.{}.session", uuid::Uuid::new_v4()));
            std::fs::rename(&legacy_path, &staging)
                .map_err(|_| "Could not stage converted session journal")?;
            drop(legacy_lease);
            if std::fs::remove_dir_all(staging).is_err() {
                crate::diagnostics::event(
                    "WARN",
                    "session.legacy_cleanup_failed",
                    json!({"sessionId":id}),
                );
            }
        }
        if let Some(turn) = journal.metadata()?.active_turn {
            journal.record(&turn,"turn_recovered",json!({"status":"interrupted","reason":"Previous writer exited before completing this turn"}))?;
        }
        Ok(journal)
    }
    fn publish(&self, metadata: &SessionMetadata, events: &[SessionEvent]) -> Result<(), String> {
        let parent = self.path.parent().ok_or("Missing session directory")?;
        let relative = parent
            .strip_prefix(&self.root)
            .map_err(|_| "Invalid session storage path")?;
        io::directory(&self.root, relative)?;
        let document = SessionDocumentRef {
            version: 2,
            metadata,
            events,
        };
        if serde_json::to_vec_pretty(&document)
            .map_err(|_| "Could not encode session JSON")?
            .len()
            .saturating_add(1)
            > self.config.max_session_bytes
        {
            return Err("Session JSON exceeds configured size limit".into());
        }
        io::write(&self.path, &document, self.config.sync_each_event)
    }
    pub fn id(&self) -> ConversationId {
        self.state
            .lock()
            .expect("session state")
            .metadata
            .id
            .clone()
    }
    pub fn path(&self) -> &Path {
        &self.path
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
            let mut candidate = state.metadata.clone();
            apply(&mut candidate, &event)?;
            state.events.push(event.clone());
            if let Err(error) = self.publish(&candidate, &state.events) {
                state.events.pop();
                return Err(error);
            }
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
        let metadata = self.metadata()?;
        let document: SessionDocument = io::read(&self.path, self.config.max_session_bytes)?;
        validate(
            &document,
            &self.config,
            &metadata.id,
            metadata.first_sent_at,
        )?;
        Ok(document.events)
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
fn validate(
    document: &SessionDocument,
    config: &SessionConfig,
    id: &ConversationId,
    first: DateTime<FixedOffset>,
) -> Result<SessionMetadata, String> {
    let mut metadata = document.metadata.clone();
    if document.version != 2
        || metadata.version != 1
        || metadata.id != *id
        || metadata.first_sent_at != first
    {
        return Err("Session metadata does not match its identity".into());
    }
    metadata.last_sequence = 0;
    metadata.active_turn = None;
    metadata.last_finished_turn = None;
    metadata.context_cursor = Default::default();
    for event in &document.events {
        if serde_json::to_vec_pretty(event)
            .map_err(|_| "Could not encode session event")?
            .len()
            .saturating_add(1)
            > config.max_event_bytes
        {
            return Err("Session event exceeds configured byte limit".into());
        }
        apply(&mut metadata, event)?;
    }
    Ok(metadata)
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
