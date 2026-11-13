//! Native owner-scoped, single-consumer live request mailboxes.
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::{
    collections::{HashMap, VecDeque},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TurnInput {
    pub id: String,
    pub content: String,
    pub created_at: DateTime<Utc>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InputReceipt {
    pub id: String,
    pub accepted: bool,
    pub finished: bool,
}
/// Byte cursor over native-sent final text; never accepts client-provided text.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DisplayCursor {
    pub response_epoch: u64,
    pub displayed_bytes: usize,
}
const DISPLAY_BYTES: usize = 16 * 1024 * 1024;
const DISPLAY_EPOCHS: usize = 64;
struct DisplayBuffer {
    epoch: u64,
    frames: VecDeque<(u64, String)>,
    bytes: usize,
    stopped: Option<Option<DisplayCursor>>,
}
impl Default for DisplayBuffer {
    fn default() -> Self {
        Self {
            epoch: 0,
            frames: VecDeque::from([(0, String::new())]),
            bytes: 0,
            stopped: None,
        }
    }
}
#[derive(Default, Clone)]
pub struct TurnManager(Arc<Mutex<HashMap<String, TurnControl>>>);
#[derive(Clone)]
pub struct TurnControl(Arc<Control>);
struct Control {
    request_id: String,
    cancelled: Arc<AtomicBool>,
    explicit_stop: AtomicBool,
    invalidated: AtomicBool,
    display: Mutex<DisplayBuffer>,
    mailbox: Mutex<Mailbox>,
}
#[derive(Default)]
struct Mailbox {
    inputs: Vec<TurnInput>,
    applied: usize,
    bytes: usize,
    finished: bool,
    consuming: bool,
}
pub struct TurnLease {
    manager: TurnManager,
    owner: String,
    control: TurnControl,
}
impl TurnManager {
    pub fn begin(&self, owner: &str) -> Result<TurnLease, String> {
        self.begin_with_cancellation(owner, Arc::new(AtomicBool::new(false)))
    }
    pub fn begin_with_cancellation(
        &self,
        owner: &str,
        cancelled: Arc<AtomicBool>,
    ) -> Result<TurnLease, String> {
        let mut turns = self.0.lock().map_err(|_| "Turn manager unavailable")?;
        if turns.contains_key(owner) {
            return Err("A request is already active for this conversation".into());
        }
        let control = TurnControl(Arc::new(Control {
            request_id: uuid::Uuid::new_v4().to_string(),
            cancelled,
            explicit_stop: AtomicBool::new(false),
            invalidated: AtomicBool::new(false),
            display: Mutex::new(DisplayBuffer::default()),
            mailbox: Mutex::new(Mailbox::default()),
        }));
        turns.insert(owner.into(), control.clone());
        Ok(TurnLease {
            manager: self.clone(),
            owner: owner.into(),
            control,
        })
    }
    fn find(&self, owner: &str, request_id: &str) -> Result<TurnControl, String> {
        self.0
            .lock()
            .map_err(|_| "Turn manager unavailable")?
            .get(owner)
            .filter(|c| c.request_id() == request_id)
            .cloned()
            .ok_or_else(|| "The request is no longer active in this conversation".into())
    }
    pub fn submit(
        &self,
        owner: &str,
        request_id: &str,
        input_id: &str,
        text: &str,
    ) -> Result<InputReceipt, String> {
        self.find(owner, request_id)?.submit(input_id, text)
    }
    pub fn cancel(&self, owner: &str, request_id: &str) -> Result<(), String> {
        self.cancel_with_cursor(owner, request_id, None)
    }
    pub fn cancel_with_cursor(
        &self,
        owner: &str,
        request_id: &str,
        cursor: Option<DisplayCursor>,
    ) -> Result<(), String> {
        let control = self.find(owner, request_id)?;
        let mut display = control
            .0
            .display
            .lock()
            .map_err(|_| "Turn display buffer unavailable")?;
        if display.stopped.is_none() {
            if cursor.as_ref().is_some_and(|c| {
                c.response_epoch > display.epoch || c.displayed_bytes > DISPLAY_BYTES
            }) {
                return Err("Displayed response cursor is out of bounds".into());
            }
            display.stopped = Some(cursor);
            control.0.explicit_stop.store(true, Ordering::Release);
        }
        drop(display);
        control.cancel();
        Ok(())
    }
    pub fn response_epoch(&self, owner: &str) -> Option<u64> {
        self.0.lock().ok()?.get(owner).map(|c| c.response_epoch())
    }
    pub fn is_cancelled(&self, owner: &str) -> bool {
        self.0
            .lock()
            .map(|turns| turns.get(owner).is_none_or(|c| c.check().is_err()))
            .unwrap_or(true)
    }
    pub fn has_pending(&self, owner: &str) -> bool {
        self.0
            .lock()
            .map(|turns| turns.get(owner).is_some_and(|c| c.has_pending()))
            .unwrap_or(false)
    }
    pub fn close_owner(&self, owner: &str) {
        if let Ok(turns) = self.0.lock() {
            if let Some(control) = turns.get(owner) {
                control.0.invalidated.store(true, Ordering::Release);
                control.cancel();
                control.seal();
            }
        }
    }
}
impl TurnLease {
    pub fn control(&self) -> TurnControl {
        self.control.clone()
    }
}
impl Drop for TurnLease {
    fn drop(&mut self) {
        self.control.seal();
        if let Ok(mut turns) = self.manager.0.lock() {
            if turns
                .get(&self.owner)
                .is_some_and(|c| c.request_id() == self.control.request_id())
            {
                turns.remove(&self.owner);
            }
        }
    }
}
pub(crate) struct TurnConsumer(TurnControl);
impl Drop for TurnConsumer {
    fn drop(&mut self) {
        self.0.seal();
    }
}
impl TurnControl {
    pub(crate) fn acquire_consumer(&self) -> Result<TurnConsumer, String> {
        let mut mailbox = self
            .0
            .mailbox
            .lock()
            .map_err(|_| "Turn mailbox unavailable")?;
        if mailbox.finished || mailbox.consuming {
            return Err("Turn is finished or already being consumed".into());
        }
        mailbox.consuming = true;
        Ok(TurnConsumer(self.clone()))
    }
    pub fn request_id(&self) -> &str {
        &self.0.request_id
    }
    pub fn user_stop_requested(&self) -> bool {
        self.0.explicit_stop.load(Ordering::Acquire) && !self.0.invalidated.load(Ordering::Acquire)
    }
    pub fn response_epoch(&self) -> u64 {
        self.0.display.lock().map(|d| d.epoch).unwrap_or(u64::MAX)
    }
    pub fn record_displayed_delta(&self, delta: &str) {
        let Ok(mut display) = self.0.display.lock() else {
            return;
        };
        let epoch = display.epoch;
        // An evicted current epoch cannot be reconstructed from a suffix.
        if !display.frames.back().is_some_and(|(id, _)| *id == epoch) {
            return;
        }
        if delta.len() > DISPLAY_BYTES {
            display.frames.clear();
            display.bytes = 0;
            return;
        }
        while display.bytes + delta.len() > DISPLAY_BYTES {
            if let Some((_, text)) = display.frames.pop_front() {
                display.bytes -= text.len();
            } else {
                break;
            }
        }
        if let Some((id, text)) = display.frames.back_mut() {
            if *id == epoch {
                text.push_str(delta);
                display.bytes += delta.len();
            }
        }
    }
    pub fn reset_displayed_output(&self) {
        let Ok(mut display) = self.0.display.lock() else {
            return;
        };
        display.epoch = display.epoch.saturating_add(1);
        let epoch = display.epoch;
        display.frames.push_back((epoch, String::new()));
        while display.frames.len() > DISPLAY_EPOCHS {
            if let Some((_, text)) = display.frames.pop_front() {
                display.bytes -= text.len();
            }
        }
    }
    pub fn stopped_prefix(&self) -> Result<Option<String>, String> {
        if !self.user_stop_requested() {
            return Ok(None);
        }
        let display = self
            .0
            .display
            .lock()
            .map_err(|_| "Turn display buffer unavailable")?;
        let Some(Some(cursor)) = &display.stopped else {
            return Ok(None);
        };
        let text = display
            .frames
            .iter()
            .find(|(id, _)| *id == cursor.response_epoch)
            .map(|(_, text)| text)
            .ok_or("Displayed response epoch is no longer retained")?;
        let prefix = text
            .get(..cursor.displayed_bytes)
            .ok_or("Displayed response cursor exceeds native-sent text or splits UTF-8")?;
        Ok(Some(prefix.into()))
    }
    pub fn cancelled(&self) -> Arc<AtomicBool> {
        self.0.cancelled.clone()
    }
    pub fn cancel(&self) {
        self.0.cancelled.store(true, Ordering::Release);
        self.seal();
    }
    pub fn check(&self) -> Result<(), String> {
        if self.0.cancelled.load(Ordering::Acquire) {
            Err("Request cancelled".into())
        } else {
            Ok(())
        }
    }
    fn seal(&self) {
        if let Ok(mut mailbox) = self.0.mailbox.lock() {
            mailbox.finished = true;
        }
    }
    pub fn submit(&self, input_id: &str, text: &str) -> Result<InputReceipt, String> {
        if input_id.is_empty()
            || input_id.len() > 128
            || text.trim().is_empty()
            || text.len() > 64 * 1024
        {
            return Err("Live input requires an ID and 1–64 KiB of text".into());
        }
        let mut mailbox = self
            .0
            .mailbox
            .lock()
            .map_err(|_| "Turn mailbox unavailable")?;
        if let Some(previous) = mailbox.inputs.iter().find(|i| i.id == input_id) {
            if previous.content != text {
                return Err("Live input ID was already used for different text".into());
            }
            return Ok(InputReceipt {
                id: input_id.into(),
                accepted: true,
                finished: mailbox.finished,
            });
        }
        if mailbox.finished {
            return Ok(InputReceipt {
                id: input_id.into(),
                accepted: false,
                finished: true,
            });
        }
        if mailbox.inputs.len() >= 32 || mailbox.bytes + text.len() > 256 * 1024 {
            return Err("Live input request limit reached".into());
        }
        mailbox.bytes += text.len();
        mailbox.inputs.push(TurnInput {
            id: input_id.into(),
            content: text.into(),
            created_at: Utc::now(),
        });
        Ok(InputReceipt {
            id: input_id.into(),
            accepted: true,
            finished: false,
        })
    }
    pub fn drain(&self) -> Vec<TurnInput> {
        let Ok(mut mailbox) = self.0.mailbox.lock() else {
            return Vec::new();
        };
        let inputs = mailbox.inputs[mailbox.applied..].to_vec();
        mailbox.applied = mailbox.inputs.len();
        inputs
    }
    pub fn applied(&self) -> Vec<TurnInput> {
        self.0
            .mailbox
            .lock()
            .map(|m| m.inputs[..m.applied].to_vec())
            .unwrap_or_default()
    }
    pub fn has_pending(&self) -> bool {
        self.0
            .mailbox
            .lock()
            .map(|m| m.applied < m.inputs.len())
            .unwrap_or(false)
    }
    pub fn try_finish(&self) -> bool {
        let Ok(mut mailbox) = self.0.mailbox.lock() else {
            return false;
        };
        if mailbox.applied < mailbox.inputs.len() || self.0.cancelled.load(Ordering::Acquire) {
            return false;
        }
        mailbox.finished = true;
        true
    }
}
