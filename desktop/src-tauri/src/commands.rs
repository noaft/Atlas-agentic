use atlas_agentic::conversations::{
    Conversation, ConversationId, ConversationMessage, ConversationMetadata, ConversationStore,
    MessageRole,
};
use serde::Serialize;
use tauri::State;

pub struct DesktopState {
    store: ConversationStore,
}

impl DesktopState {
    pub fn new(store: ConversationStore) -> Self {
        Self { store }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConversationMetadataDto {
    pub id: String,
    pub title: String,
    pub title_attempted: bool,
    pub created_at: String,
    pub updated_at: String,
    pub message_count: usize,
}

impl From<ConversationMetadata> for ConversationMetadataDto {
    fn from(metadata: ConversationMetadata) -> Self {
        Self {
            id: metadata.id.to_string(),
            title: metadata.title,
            title_attempted: metadata.title_attempted,
            created_at: metadata.created_at.to_rfc3339(),
            updated_at: metadata.updated_at.to_rfc3339(),
            message_count: metadata.message_count,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConversationMessageDto {
    pub role: &'static str,
    pub content: String,
    pub created_at: String,
}

impl From<ConversationMessage> for ConversationMessageDto {
    fn from(message: ConversationMessage) -> Self {
        Self {
            role: match message.role {
                MessageRole::User => "user",
                MessageRole::Assistant => "assistant",
            },
            content: message.content,
            created_at: message.created_at.to_rfc3339(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConversationDto {
    pub metadata: ConversationMetadataDto,
    pub messages: Vec<ConversationMessageDto>,
}

impl From<Conversation> for ConversationDto {
    fn from(conversation: Conversation) -> Self {
        Self {
            metadata: conversation.metadata.into(),
            messages: conversation.messages.into_iter().map(Into::into).collect(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DeleteResult {
    pub id: String,
}

fn parse_id(id: &str) -> Result<ConversationId, String> {
    ConversationId::parse(id)
}

fn list_impl(state: &DesktopState) -> Result<Vec<ConversationMetadataDto>, String> {
    state
        .store
        .list()
        .map(|items| items.into_iter().map(Into::into).collect())
}

fn load_impl(id: &str, state: &DesktopState) -> Result<ConversationDto, String> {
    state.store.load(&parse_id(id)?).map(Into::into)
}

fn rename_impl(
    id: &str,
    title: &str,
    state: &DesktopState,
) -> Result<ConversationMetadataDto, String> {
    let title = title.trim();
    if title.is_empty() {
        return Err("conversation title cannot be empty".to_string());
    }
    state.store.rename(&parse_id(id)?, title).map(Into::into)
}

fn delete_impl(id: &str, state: &DesktopState) -> Result<DeleteResult, String> {
    let id = parse_id(id)?;
    state.store.delete(&id)?;
    Ok(DeleteResult { id: id.to_string() })
}

#[tauri::command]
pub fn list_conversations(
    state: State<'_, DesktopState>,
) -> Result<Vec<ConversationMetadataDto>, String> {
    list_impl(&state)
}

#[tauri::command]
pub fn load_conversation(
    id: String,
    state: State<'_, DesktopState>,
) -> Result<ConversationDto, String> {
    load_impl(&id, &state)
}

#[tauri::command]
pub fn rename_conversation(
    id: String,
    title: String,
    state: State<'_, DesktopState>,
) -> Result<ConversationMetadataDto, String> {
    rename_impl(&id, &title, &state)
}

#[tauri::command]
pub fn delete_conversation(
    id: String,
    state: State<'_, DesktopState>,
) -> Result<DeleteResult, String> {
    delete_impl(&id, &state)
}

#[cfg(test)]
mod tests {
    use atlas_agentic::conversations::ConversationStore;
    use tempfile::TempDir;

    use super::{delete_impl, list_impl, load_impl, rename_impl, DesktopState};

    fn fixture() -> (TempDir, ConversationStore, DesktopState) {
        let data = TempDir::new().unwrap();
        let store = ConversationStore::at(data.path().to_path_buf());
        let state = DesktopState::new(store.clone());
        (data, store, state)
    }

    #[test]
    fn crud_does_not_need_provider_configuration() {
        let (_data, store, state) = fixture();
        let saved = store.create_turn("question", "answer").unwrap();

        let listed = list_impl(&state).unwrap();
        let loaded = load_impl(&saved.id.to_string(), &state).unwrap();
        let renamed = rename_impl(&saved.id.to_string(), "New title", &state).unwrap();

        assert_eq!(listed.len(), 1);
        assert_eq!(loaded.metadata.id, saved.id.to_string());
        assert_eq!(renamed.title, "New title");
    }

    #[test]
    fn list_returns_most_recent_first() {
        let (_data, store, state) = fixture();
        let older = store.create_turn("older", "answer").unwrap();
        let newer = store.create_turn("newer", "answer").unwrap();

        let listed = list_impl(&state).unwrap();

        assert_eq!(listed.len(), 2);
        assert_eq!(listed[0].id, newer.id.to_string());
        assert_eq!(listed[1].id, older.id.to_string());
    }

    #[test]
    fn load_rejects_invalid_id() {
        let (_data, _store, state) = fixture();

        let error = load_impl("not-an-id", &state).unwrap_err();

        assert!(error.contains("invalid conversation ID"), "{error}");
    }

    #[test]
    fn rename_returns_updated_metadata_and_preserves_history() {
        let (_data, store, state) = fixture();
        let saved = store.create_turn("question", "answer").unwrap();

        let renamed = rename_impl(&saved.id.to_string(), "  Desktop work  ", &state).unwrap();
        let loaded = store.load(&saved.id).unwrap();

        assert_eq!(renamed.title, "Desktop work");
        assert_eq!(loaded.metadata.title, "Desktop work");
        assert_eq!(loaded.messages.len(), 2);
        assert_eq!(loaded.messages[0].content, "question");
        assert_eq!(loaded.messages[1].content, "answer");
    }

    #[test]
    fn delete_returns_id_and_removes_conversation() {
        let (_data, store, state) = fixture();
        let saved = store.create_turn("question", "answer").unwrap();

        let deleted = delete_impl(&saved.id.to_string(), &state).unwrap();

        assert_eq!(deleted.id, saved.id.to_string());
        assert!(store.list().unwrap().is_empty());
        assert!(store.load(&saved.id).is_err());
    }
}
