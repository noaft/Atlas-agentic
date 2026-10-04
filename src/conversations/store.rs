use std::{
    env, fmt,
    fs::{self, File, OpenOptions},
    io::{self, BufRead, BufReader, BufWriter, Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    sync::Arc,
};

use chrono::Utc;
use directories::BaseDirs;
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::{Conversation, ConversationId, ConversationMessage, ConversationMetadata, MessageRole};

const INDEX_VERSION: u32 = 1;
const FALLBACK_TITLE: &str = "Untitled conversation";
type IndexPublisher = dyn Fn(&Path, &Path) -> io::Result<()> + Send + Sync;

#[derive(Debug, Serialize, Deserialize)]
struct ConversationIndex {
    version: u32,
    conversations: Vec<ConversationMetadata>,
}

impl Default for ConversationIndex {
    fn default() -> Self {
        Self {
            version: INDEX_VERSION,
            conversations: Vec::new(),
        }
    }
}

#[derive(Clone)]
pub struct ConversationStore {
    root: PathBuf,
    index_publisher: Arc<IndexPublisher>,
}

impl fmt::Debug for ConversationStore {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ConversationStore")
            .field("root", &self.root)
            .finish_non_exhaustive()
    }
}

impl ConversationStore {
    pub fn from_user_data() -> Result<Self, String> {
        if let Some(root) = env::var_os("ATLAS_DATA_DIR") {
            return Ok(Self::at(PathBuf::from(root)));
        }

        let root = BaseDirs::new()
            .ok_or_else(|| "could not resolve the user-local data directory".to_string())?
            .data_local_dir()
            .join("atlas");
        Ok(Self::at(root))
    }

    pub fn at(root: PathBuf) -> Self {
        Self::at_with_index_publisher(root, |source, destination| fs::rename(source, destination))
    }

    #[doc(hidden)]
    pub fn at_with_index_publisher(
        root: PathBuf,
        index_publisher: impl Fn(&Path, &Path) -> io::Result<()> + Send + Sync + 'static,
    ) -> Self {
        Self {
            root,
            index_publisher: Arc::new(index_publisher),
        }
    }

    pub fn create_turn(&self, user: &str, assistant: &str) -> Result<ConversationMetadata, String> {
        self.with_exclusive_lock(|| {
            let mut index = self.read_index()?;
            fs::create_dir_all(self.conversations_dir()).map_err(|error| {
                format!(
                    "failed to create {}: {error}",
                    self.conversations_dir().display()
                )
            })?;

            let id = loop {
                let candidate = ConversationId::new();
                if !index
                    .conversations
                    .iter()
                    .any(|metadata| metadata.id == candidate)
                    && !self.history_path(&candidate).exists()
                {
                    break candidate;
                }
            };
            let now = Utc::now();
            let messages = [
                ConversationMessage {
                    role: MessageRole::User,
                    content: user.to_string(),
                    created_at: now,
                },
                ConversationMessage {
                    role: MessageRole::Assistant,
                    content: assistant.to_string(),
                    created_at: now,
                },
            ];
            let history_path = self.history_path(&id);
            self.write_new_history(&history_path, &messages)?;

            let metadata = ConversationMetadata {
                id,
                title: FALLBACK_TITLE.to_string(),
                title_attempted: false,
                created_at: now,
                updated_at: now,
                message_count: messages.len(),
            };
            index.conversations.push(metadata.clone());
            if let Err(error) = self.write_index(&index) {
                return Err(format!(
                    "{error}; conversation history preserved at {}",
                    history_path.display()
                ));
            }
            Ok(metadata)
        })
    }

    pub fn append_turn(
        &self,
        id: &ConversationId,
        user: &str,
        assistant: &str,
    ) -> Result<ConversationMetadata, String> {
        self.with_exclusive_lock(|| {
            let mut index = self.read_index()?;
            let position = find_metadata_position(&index, id)?;
            let history_path = self.history_path(id);
            let existing_messages = self.read_history(&history_path)?;
            reconcile_metadata(&mut index.conversations[position], &existing_messages);

            let now = Utc::now();
            let messages = [
                ConversationMessage {
                    role: MessageRole::User,
                    content: user.to_string(),
                    created_at: now,
                },
                ConversationMessage {
                    role: MessageRole::Assistant,
                    content: assistant.to_string(),
                    created_at: now,
                },
            ];
            self.append_history(&history_path, &messages)?;

            let metadata = &mut index.conversations[position];
            metadata.message_count = existing_messages.len() + messages.len();
            metadata.updated_at = now;
            let metadata = metadata.clone();
            self.write_index(&index)?;
            Ok(metadata)
        })
    }

    pub fn load(&self, id: &ConversationId) -> Result<Conversation, String> {
        self.with_shared_lock(|| {
            let index = self.read_index()?;
            let position = find_metadata_position(&index, id)?;
            let messages = self.read_history(&self.history_path(id))?;
            let mut metadata = index.conversations[position].clone();
            reconcile_metadata(&mut metadata, &messages);
            Ok(Conversation { metadata, messages })
        })
    }

    pub fn list(&self) -> Result<Vec<ConversationMetadata>, String> {
        let mut conversations = self.read_index()?.conversations;
        conversations.sort_by(|left, right| {
            right
                .updated_at
                .cmp(&left.updated_at)
                .then_with(|| right.created_at.cmp(&left.created_at))
                .then_with(|| left.id.to_string().cmp(&right.id.to_string()))
        });
        Ok(conversations)
    }

    pub fn rename(&self, id: &ConversationId, title: &str) -> Result<ConversationMetadata, String> {
        self.update_metadata(id, |metadata| metadata.title = title.to_string())
    }

    pub fn record_title_attempt(
        &self,
        id: &ConversationId,
        title: Option<&str>,
    ) -> Result<ConversationMetadata, String> {
        self.update_metadata(id, |metadata| {
            if let Some(title) = title {
                metadata.title = title.to_string();
            }
            metadata.title_attempted = true;
        })
    }

    pub fn delete(&self, id: &ConversationId) -> Result<PathBuf, String> {
        self.with_exclusive_lock(|| {
            let mut index = self.read_index()?;
            let position = find_metadata_position(&index, id)?;
            let history_path = self.history_path(id);
            let trash_dir = self.root.join("trash");
            fs::create_dir_all(&trash_dir)
                .map_err(|error| format!("failed to create {}: {error}", trash_dir.display()))?;
            let trash_path = trash_dir.join(format!("{id}.jsonl"));
            fs::rename(&history_path, &trash_path).map_err(|error| {
                format!(
                    "failed to move {} to {}: {error}",
                    history_path.display(),
                    trash_path.display()
                )
            })?;

            index.conversations.remove(position);
            if let Err(publication_error) = self.write_index(&index) {
                return match fs::rename(&trash_path, &history_path) {
                    Ok(()) => Err(format!(
                        "{publication_error}; deletion rolled back to {}",
                        history_path.display()
                    )),
                    Err(rollback_error) => Err(format!(
                        "{publication_error}; failed to roll back deletion from {} to {}: \
                         {rollback_error}; conversation history remains recoverable at {}",
                        trash_path.display(),
                        history_path.display(),
                        trash_path.display()
                    )),
                };
            }
            Ok(trash_path)
        })
    }

    fn update_metadata(
        &self,
        id: &ConversationId,
        update: impl FnOnce(&mut ConversationMetadata),
    ) -> Result<ConversationMetadata, String> {
        self.with_exclusive_lock(|| {
            let mut index = self.read_index()?;
            let position = find_metadata_position(&index, id)?;
            let messages = self.read_history(&self.history_path(id))?;
            let metadata = &mut index.conversations[position];
            reconcile_metadata(metadata, &messages);
            update(metadata);
            let metadata = metadata.clone();
            self.write_index(&index)?;
            Ok(metadata)
        })
    }

    fn with_exclusive_lock<T>(
        &self,
        operation: impl FnOnce() -> Result<T, String>,
    ) -> Result<T, String> {
        let lock_file = self.open_lock()?;
        lock_file
            .lock_exclusive()
            .map_err(|error| format!("failed to lock {}: {error}", self.lock_path().display()))?;
        operation()
    }

    fn with_shared_lock<T>(
        &self,
        operation: impl FnOnce() -> Result<T, String>,
    ) -> Result<T, String> {
        let lock_file = self.open_lock()?;
        FileExt::lock_shared(&lock_file)
            .map_err(|error| format!("failed to lock {}: {error}", self.lock_path().display()))?;
        operation()
    }

    fn open_lock(&self) -> Result<File, String> {
        fs::create_dir_all(&self.root)
            .map_err(|error| format!("failed to create {}: {error}", self.root.display()))?;
        let lock_path = self.lock_path();
        OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .truncate(false)
            .open(&lock_path)
            .map_err(|error| format!("failed to open {}: {error}", lock_path.display()))
    }

    fn read_index(&self) -> Result<ConversationIndex, String> {
        let path = self.index_path();
        let contents = match fs::read_to_string(&path) {
            Ok(contents) => contents,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(ConversationIndex::default())
            }
            Err(error) => return Err(format!("failed to read {}: {error}", path.display())),
        };
        let index: ConversationIndex = serde_json::from_str(&contents)
            .map_err(|error| format!("failed to parse {}: {error}", path.display()))?;
        if index.version != INDEX_VERSION {
            return Err(format!(
                "failed to parse {}: unsupported version {}",
                path.display(),
                index.version
            ));
        }
        Ok(index)
    }

    fn write_index(&self, index: &ConversationIndex) -> Result<(), String> {
        let index_path = self.index_path();
        let temporary_path = self
            .root
            .join(format!(".conversations.json.{}.tmp", Uuid::new_v4()));
        let result = (|| {
            let file = OpenOptions::new()
                .create_new(true)
                .write(true)
                .open(&temporary_path)
                .map_err(|error| {
                    format!("failed to create {}: {error}", temporary_path.display())
                })?;
            let mut writer = BufWriter::new(file);
            serde_json::to_writer_pretty(&mut writer, index).map_err(|error| {
                format!("failed to serialize {}: {error}", index_path.display())
            })?;
            writer.write_all(b"\n").map_err(|error| {
                format!("failed to write {}: {error}", temporary_path.display())
            })?;
            writer.flush().map_err(|error| {
                format!("failed to flush {}: {error}", temporary_path.display())
            })?;
            drop(writer);
            (self.index_publisher)(&temporary_path, &index_path).map_err(|error| {
                format!(
                    "failed to replace {} with {}: {error}",
                    index_path.display(),
                    temporary_path.display()
                )
            })
        })();
        if result.is_err() {
            let _ = fs::remove_file(&temporary_path);
        }
        result
    }

    fn write_new_history(
        &self,
        path: &Path,
        messages: &[ConversationMessage],
    ) -> Result<(), String> {
        let file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(path)
            .map_err(|error| format!("failed to create {}: {error}", path.display()))?;
        write_messages(file, path, messages)
    }

    fn append_history(&self, path: &Path, messages: &[ConversationMessage]) -> Result<(), String> {
        let mut file = OpenOptions::new()
            .read(true)
            .append(true)
            .open(path)
            .map_err(|error| format!("failed to open {}: {error}", path.display()))?;
        if file
            .metadata()
            .map_err(|error| format!("failed to inspect {}: {error}", path.display()))?
            .len()
            > 0
        {
            file.seek(SeekFrom::End(-1))
                .map_err(|error| format!("failed to inspect {}: {error}", path.display()))?;
            let mut last_byte = [0];
            file.read_exact(&mut last_byte)
                .map_err(|error| format!("failed to inspect {}: {error}", path.display()))?;
            if last_byte[0] != b'\n' {
                file.write_all(b"\n")
                    .map_err(|error| format!("failed to write {}: {error}", path.display()))?;
            }
        }
        write_messages(file, path, messages)
    }

    fn read_history(&self, path: &Path) -> Result<Vec<ConversationMessage>, String> {
        let file = File::open(path)
            .map_err(|error| format!("failed to read {}: {error}", path.display()))?;
        BufReader::new(file)
            .lines()
            .enumerate()
            .map(|(line_number, line)| {
                let line = line.map_err(|error| {
                    format!(
                        "failed to read {} line {}: {error}",
                        path.display(),
                        line_number + 1
                    )
                })?;
                serde_json::from_str(&line).map_err(|error| {
                    format!(
                        "failed to parse {} line {}: {error}",
                        path.display(),
                        line_number + 1
                    )
                })
            })
            .collect()
    }

    fn index_path(&self) -> PathBuf {
        self.root.join("conversations.json")
    }

    fn lock_path(&self) -> PathBuf {
        self.root.join("conversations.lock")
    }

    fn conversations_dir(&self) -> PathBuf {
        self.root.join("conversations")
    }

    fn history_path(&self, id: &ConversationId) -> PathBuf {
        self.conversations_dir().join(format!("{id}.jsonl"))
    }
}

fn write_messages(file: File, path: &Path, messages: &[ConversationMessage]) -> Result<(), String> {
    let mut writer = BufWriter::new(file);
    for message in messages {
        serde_json::to_writer(&mut writer, message)
            .map_err(|error| format!("failed to serialize {}: {error}", path.display()))?;
        writer
            .write_all(b"\n")
            .map_err(|error| format!("failed to write {}: {error}", path.display()))?;
    }
    writer
        .flush()
        .map_err(|error| format!("failed to flush {}: {error}", path.display()))
}

fn find_metadata_position(index: &ConversationIndex, id: &ConversationId) -> Result<usize, String> {
    index
        .conversations
        .iter()
        .position(|metadata| &metadata.id == id)
        .ok_or_else(|| format!("conversation not found: {id}"))
}

fn reconcile_metadata(metadata: &mut ConversationMetadata, messages: &[ConversationMessage]) {
    metadata.message_count = messages.len();
    if let Some(message) = messages.last() {
        metadata.updated_at = message.created_at;
    }
}
