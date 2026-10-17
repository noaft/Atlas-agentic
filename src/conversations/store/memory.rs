use super::*;
use crate::conversations::{memory::fingerprint, MemoryCheckpoint};

pub struct ConversationLease {
    _file: File,
}
impl ConversationStore {
    pub fn acquire_execution(&self, id: &ConversationId) -> Result<ConversationLease, String> {
        let directory = self.root.join("leases");
        create_private_dir_all(&directory).map_err(|e| e.to_string())?;
        let file = private_file_options()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(directory.join(format!("{id}.lock")))
            .map_err(|e| e.to_string())?;
        file.try_lock_exclusive()
            .map_err(|_| "This conversation is already running an ask or compaction".to_string())?;
        Ok(ConversationLease { _file: file })
    }
    pub fn load_memory(&self, id: &ConversationId) -> Result<Option<MemoryCheckpoint>, String> {
        self.with_shared_lock(|| {
            find_metadata_position(&self.read_index()?, id)?;
            self.read_memory(id, &self.read_history(&self.history_path(id))?)
        })
    }
    pub(super) fn read_memory(
        &self,
        id: &ConversationId,
        messages: &[ConversationMessage],
    ) -> Result<Option<MemoryCheckpoint>, String> {
        let path = self.memory_path(id);
        let bytes = match fs::read(&path) {
            Ok(b) => b,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(format!("Cannot read memory: {e}")),
        };
        let checkpoint: MemoryCheckpoint = serde_json::from_slice(&bytes)
            .map_err(|e| format!("Invalid memory checkpoint: {e}"))?;
        checkpoint.validate(id, messages)?;
        Ok(Some(checkpoint))
    }
    pub fn replace_memory(
        &self,
        id: &ConversationId,
        expected: Option<&str>,
        checkpoint: &MemoryCheckpoint,
    ) -> Result<(), String> {
        self.with_exclusive_lock(|| {
            find_metadata_position(&self.read_index()?, id)?;
            let messages = self.read_history(&self.history_path(id))?;
            checkpoint.validate(id, &messages)?;
            let current = self.read_memory(id, &messages)?;
            if current.as_ref().map(|m| m.checkpoint_id.as_str()) != expected {
                return Err("Conversation memory changed while compacting; retry".into());
            }
            let path = self.memory_path(id);
            create_private_dir_all(path.parent().unwrap()).map_err(|e| e.to_string())?;
            let temporary = path.with_extension(format!("{}.tmp", uuid::Uuid::new_v4()));
            let result = (|| {
                let mut file = private_file_options()
                    .create_new(true)
                    .write(true)
                    .open(&temporary)
                    .map_err(|e| e.to_string())?;
                serde_json::to_writer_pretty(&mut file, checkpoint).map_err(|e| e.to_string())?;
                file.write_all(b"\n")
                    .and_then(|()| file.sync_all())
                    .map_err(|e| e.to_string())?;
                drop(file);
                (self.index_publisher)(&temporary, &path)
                    .map_err(|e| format!("Cannot publish memory: {e}"))
            })();
            if result.is_err() {
                let _ = fs::remove_file(&temporary);
            }
            result
        })
    }
    pub(super) fn memory_path(&self, id: &ConversationId) -> PathBuf {
        self.root.join("memory").join(format!("{id}.json"))
    }
}
pub fn memory_fingerprint(messages: &[ConversationMessage]) -> Result<String, String> {
    fingerprint(messages)
}
