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

use crate::permissions::PermissionMode;

#[cfg(unix)]
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};

use super::{
    ActivityTurn, Conversation, ConversationId, ConversationMessage, ConversationMetadata,
    MessageRole, ACTIVITY_VERSION,
};

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
        self.create_turn_with_workspace(user, assistant, None)
    }

    pub fn create_turn_with_workspace(
        &self,
        user: &str,
        assistant: &str,
        workspace: Option<&Path>,
    ) -> Result<ConversationMetadata, String> {
        self.create_turn_with_workspace_and_permission(user, assistant, workspace, None)
    }

    pub fn create_turn_with_workspace_and_permission(
        &self,
        user: &str,
        assistant: &str,
        workspace: Option<&Path>,
        permission_mode: Option<PermissionMode>,
    ) -> Result<ConversationMetadata, String> {
        self.create_turn_inner(user, assistant, workspace, permission_mode, None)
    }

    pub fn create_turn_with_activity(
        &self,
        user: &str,
        assistant: &str,
        workspace: Option<&Path>,
        permission_mode: Option<PermissionMode>,
        activity: ActivityTurn,
    ) -> Result<ConversationMetadata, String> {
        self.create_turn_inner(user, assistant, workspace, permission_mode, Some(activity))
    }

    fn create_turn_inner(
        &self,
        user: &str,
        assistant: &str,
        workspace: Option<&Path>,
        permission_mode: Option<PermissionMode>,
        activity: Option<ActivityTurn>,
    ) -> Result<ConversationMetadata, String> {
        self.with_exclusive_lock(|| {
            let mut index = self.read_index()?;
            let workspace_path = workspace.map(validate_workspace).transpose()?;
            create_private_dir_all(&self.conversations_dir()).map_err(|error| {
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
            let activity_path = self.activity_path(&id);
            if let Some(mut activity) = activity {
                activity.assistant_message_index = 1;
                create_private_dir_all(&self.activities_dir()).map_err(|error| {
                    format!(
                        "failed to create {}: {error}",
                        self.activities_dir().display()
                    )
                })?;
                self.write_new_activity(&activity_path, &activity)?;
            }
            if let Err(error) = self.write_new_history(&history_path, &messages) {
                if activity_path.exists() {
                    return match fs::remove_file(&activity_path) {
                        Ok(()) => Err(format!("{error}; activity creation rolled back")),
                        Err(rollback_error) => Err(format!(
                            "{error}; failed to remove {}: {rollback_error}",
                            activity_path.display()
                        )),
                    };
                }
                return Err(error);
            }

            let metadata = ConversationMetadata {
                id,
                title: FALLBACK_TITLE.to_string(),
                title_attempted: false,
                created_at: now,
                updated_at: now,
                message_count: messages.len(),
                workspace_path,
                permission_mode,
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

    pub fn assign_workspace(
        &self,
        id: &ConversationId,
        workspace: &Path,
    ) -> Result<ConversationMetadata, String> {
        self.with_exclusive_lock(|| {
            let workspace = validate_workspace(workspace)?;
            let mut index = self.read_index()?;
            let position = find_metadata_position(&index, id)?;
            let messages = self.read_history(&self.history_path(id))?;
            let metadata = &mut index.conversations[position];
            reconcile_metadata(metadata, &messages);
            metadata.workspace_path = Some(workspace);
            metadata.updated_at = Utc::now();
            let metadata = metadata.clone();
            self.write_index(&index)?;
            Ok(metadata)
        })
    }

    pub fn set_permission_mode(
        &self,
        id: &ConversationId,
        permission_mode: PermissionMode,
    ) -> Result<ConversationMetadata, String> {
        self.update_metadata(id, |metadata| {
            metadata.permission_mode = Some(permission_mode);
        })
    }

    pub fn append_turn(
        &self,
        id: &ConversationId,
        user: &str,
        assistant: &str,
    ) -> Result<ConversationMetadata, String> {
        self.append_turn_with_workspace_if_missing(id, user, assistant, None)
    }

    pub fn append_turn_with_workspace_if_missing(
        &self,
        id: &ConversationId,
        user: &str,
        assistant: &str,
        workspace: Option<&Path>,
    ) -> Result<ConversationMetadata, String> {
        self.append_turn_with_workspace_and_permission_if_missing(
            id, user, assistant, workspace, None,
        )
    }

    pub fn append_turn_with_workspace_and_permission_if_missing(
        &self,
        id: &ConversationId,
        user: &str,
        assistant: &str,
        workspace: Option<&Path>,
        permission_mode: Option<PermissionMode>,
    ) -> Result<ConversationMetadata, String> {
        self.append_turn_inner(id, user, assistant, workspace, permission_mode, None)
    }

    pub fn append_turn_with_activity(
        &self,
        id: &ConversationId,
        user: &str,
        assistant: &str,
        workspace: Option<&Path>,
        permission_mode: Option<PermissionMode>,
        activity: ActivityTurn,
    ) -> Result<ConversationMetadata, String> {
        self.append_turn_inner(
            id,
            user,
            assistant,
            workspace,
            permission_mode,
            Some(activity),
        )
    }

    fn append_turn_inner(
        &self,
        id: &ConversationId,
        user: &str,
        assistant: &str,
        workspace: Option<&Path>,
        permission_mode: Option<PermissionMode>,
        activity: Option<ActivityTurn>,
    ) -> Result<ConversationMetadata, String> {
        self.with_exclusive_lock(|| {
            let mut index = self.read_index()?;
            let position = find_metadata_position(&index, id)?;
            let history_path = self.history_path(id);
            let existing_messages = self.read_history(&history_path)?;
            reconcile_metadata(&mut index.conversations[position], &existing_messages);
            let workspace_path = if index.conversations[position].workspace_path.is_none() {
                workspace.map(validate_workspace).transpose()?
            } else {
                None
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
            let activity_path = self.activity_path(id);
            let activity_original_len = if let Some(mut activity) = activity {
                activity.assistant_message_index = existing_messages.len() + 1;
                create_private_dir_all(&self.activities_dir()).map_err(|error| {
                    format!(
                        "failed to create {}: {error}",
                        self.activities_dir().display()
                    )
                })?;
                Some(self.append_activity(&activity_path, &activity)?)
            } else {
                None
            };
            if let Err(error) = self.append_history(&history_path, &messages) {
                if let Some(original_len) = activity_original_len {
                    return match rollback_file(&activity_path, original_len) {
                        Ok(()) => Err(format!("{error}; activity append rolled back")),
                        Err(rollback_error) => Err(format!(
                            "{error}; failed to roll back {} to {original_len} bytes: {rollback_error}",
                            activity_path.display()
                        )),
                    };
                }
                return Err(error);
            }

            let metadata = &mut index.conversations[position];
            if metadata.workspace_path.is_none() {
                metadata.workspace_path = workspace_path;
            }
            if metadata.permission_mode.is_none() {
                metadata.permission_mode = permission_mode;
            }
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
            let activities = self.read_activities(&self.activity_path(id), messages.len())?;
            let mut metadata = index.conversations[position].clone();
            reconcile_metadata(&mut metadata, &messages);
            Ok(Conversation {
                metadata,
                messages,
                activities,
            })
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
        self.update_metadata(id, |metadata| {
            metadata.title = title.to_string();
            // A manual title also closes any in-flight automatic title attempt.
            metadata.title_attempted = true;
        })
    }

    pub fn record_title_attempt(
        &self,
        id: &ConversationId,
        title: Option<&str>,
    ) -> Result<ConversationMetadata, String> {
        self.update_metadata(id, |metadata| {
            if !metadata.title_attempted {
                if let Some(title) = title {
                    metadata.title = title.to_string();
                }
            }
            metadata.title_attempted = true;
        })
    }

    pub fn delete(&self, id: &ConversationId) -> Result<PathBuf, String> {
        self.with_exclusive_lock(|| {
            let mut index = self.read_index()?;
            let position = find_metadata_position(&index, id)?;
            let history_path = self.history_path(id);
            let activity_path = self.activity_path(id);
            let trash_dir = self.root.join("trash");
            create_private_dir_all(&trash_dir)
                .map_err(|error| format!("failed to create {}: {error}", trash_dir.display()))?;
            let trash_path = trash_dir.join(format!("{id}.jsonl"));
            let activity_trash_path = trash_dir.join(format!("{id}.activity.jsonl"));
            fs::rename(&history_path, &trash_path).map_err(|error| {
                format!(
                    "failed to move {} to {}: {error}",
                    history_path.display(),
                    trash_path.display()
                )
            })?;
            let moved_activity = if activity_path.exists() {
                if let Err(error) = fs::rename(&activity_path, &activity_trash_path) {
                    return match fs::rename(&trash_path, &history_path) {
                        Ok(()) => Err(format!(
                            "failed to move {} to {}: {error}; history deletion rolled back",
                            activity_path.display(),
                            activity_trash_path.display()
                        )),
                        Err(rollback_error) => Err(format!(
                            "failed to move {} to {}: {error}; failed to restore {}: {rollback_error}",
                            activity_path.display(),
                            activity_trash_path.display(),
                            history_path.display()
                        )),
                    };
                }
                true
            } else {
                false
            };

            index.conversations.remove(position);
            if let Err(publication_error) = self.write_index(&index) {
                let activity_rollback = if moved_activity {
                    fs::rename(&activity_trash_path, &activity_path)
                } else {
                    Ok(())
                };
                let history_rollback = fs::rename(&trash_path, &history_path);
                return match (history_rollback, activity_rollback) {
                    (Ok(()), Ok(())) => Err(format!(
                        "{publication_error}; deletion rolled back to {}",
                        history_path.display()
                    )),
                    (history_result, activity_result) => Err(format!(
                        "{publication_error}; deletion rollback incomplete: history={history_result:?}, activity={activity_result:?}; history remains recoverable at {}",
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
        create_private_dir_all(&self.root)
            .map_err(|error| format!("failed to create {}: {error}", self.root.display()))?;
        let lock_path = self.lock_path();
        private_file_options()
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
            let file = private_file_options()
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
        let file = private_file_options()
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
        let original_len = file
            .metadata()
            .map_err(|error| format!("failed to inspect {}: {error}", path.display()))?
            .len();
        let result = (|| {
            if original_len > 0 {
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
            write_messages(&mut file, path, messages)
        })();
        if let Err(error) = result {
            // write_messages has dropped its buffered writer before rollback, so
            // no pending bytes can be flushed after we restore the history length.
            return match file.set_len(original_len) {
                Ok(()) => Err(format!("{error}; history append rolled back")),
                Err(rollback_error) => Err(format!(
                    "{error}; failed to roll back {} to {original_len} bytes: {rollback_error}",
                    path.display()
                )),
            };
        }
        Ok(())
    }

    fn write_new_activity(&self, path: &Path, activity: &ActivityTurn) -> Result<(), String> {
        let file = private_file_options()
            .create_new(true)
            .write(true)
            .open(path)
            .map_err(|error| format!("failed to create {}: {error}", path.display()))?;
        write_activity(file, path, activity)
    }

    fn append_activity(&self, path: &Path, activity: &ActivityTurn) -> Result<u64, String> {
        let mut file = private_file_options()
            .create(true)
            .read(true)
            .append(true)
            .open(path)
            .map_err(|error| format!("failed to open {}: {error}", path.display()))?;
        let original_len = file
            .metadata()
            .map_err(|error| format!("failed to inspect {}: {error}", path.display()))?
            .len();
        let result = (|| {
            if original_len > 0 {
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
            write_activity(&mut file, path, activity)
        })();
        if let Err(error) = result {
            return match rollback_file(path, original_len) {
                Ok(()) => Err(format!("{error}; activity append rolled back")),
                Err(rollback_error) => Err(format!(
                    "{error}; failed to roll back {} to {original_len} bytes: {rollback_error}",
                    path.display()
                )),
            };
        }
        Ok(original_len)
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

    fn read_activities(
        &self,
        path: &Path,
        durable_message_count: usize,
    ) -> Result<Vec<ActivityTurn>, String> {
        let file = match File::open(path) {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => return Err(format!("failed to read {}: {error}", path.display())),
        };
        let mut activities = Vec::new();
        for (line_number, line) in BufReader::new(file).lines().enumerate() {
            let line = line.map_err(|error| {
                format!(
                    "failed to read {} line {}: {error}",
                    path.display(),
                    line_number + 1
                )
            })?;
            let activity: ActivityTurn = serde_json::from_str(&line).map_err(|error| {
                format!(
                    "failed to parse {} line {}: {error}",
                    path.display(),
                    line_number + 1
                )
            })?;
            if activity.version != ACTIVITY_VERSION {
                return Err(format!(
                    "failed to parse {} line {}: unsupported version {}",
                    path.display(),
                    line_number + 1,
                    activity.version
                ));
            }
            if activity.assistant_message_index < durable_message_count {
                activities.push(activity);
            }
        }
        Ok(activities)
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

    fn activities_dir(&self) -> PathBuf {
        self.root.join("activities")
    }

    fn history_path(&self, id: &ConversationId) -> PathBuf {
        self.conversations_dir().join(format!("{id}.jsonl"))
    }

    fn activity_path(&self, id: &ConversationId) -> PathBuf {
        self.activities_dir().join(format!("{id}.jsonl"))
    }
}

pub fn validate_workspace(path: &Path) -> Result<PathBuf, String> {
    let canonical = fs::canonicalize(path).map_err(|error| {
        format!(
            "workspace does not exist or cannot be accessed ({}): {error}",
            path.display()
        )
    })?;
    if !canonical.is_dir() {
        return Err(format!(
            "workspace must be an existing directory: {}",
            path.display()
        ));
    }
    if canonical.to_str().is_none() {
        return Err("workspace path must contain valid Unicode".to_string());
    }
    Ok(canonical)
}

fn create_private_dir_all(path: &Path) -> io::Result<()> {
    let mut builder = fs::DirBuilder::new();
    builder.recursive(true);
    // Creation modes never change an existing user-supplied data root.
    #[cfg(unix)]
    builder.mode(0o700);
    builder.create(path)
}

fn private_file_options() -> OpenOptions {
    #[cfg(unix)]
    {
        let mut options = OpenOptions::new();
        options.mode(0o600);
        options
    }
    #[cfg(not(unix))]
    OpenOptions::new()
}

fn write_messages(
    file: impl Write,
    path: &Path,
    messages: &[ConversationMessage],
) -> Result<(), String> {
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

fn write_activity(file: impl Write, path: &Path, activity: &ActivityTurn) -> Result<(), String> {
    let mut writer = BufWriter::new(file);
    serde_json::to_writer(&mut writer, activity)
        .map_err(|error| format!("failed to serialize {}: {error}", path.display()))?;
    writer
        .write_all(b"\n")
        .map_err(|error| format!("failed to write {}: {error}", path.display()))?;
    writer
        .flush()
        .map_err(|error| format!("failed to flush {}: {error}", path.display()))
}

fn rollback_file(path: &Path, original_len: u64) -> io::Result<()> {
    if original_len == 0 {
        fs::remove_file(path)
    } else {
        OpenOptions::new()
            .write(true)
            .open(path)?
            .set_len(original_len)
    }
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
        metadata.updated_at = metadata.updated_at.max(message.created_at);
    }
}
