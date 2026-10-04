use std::{path::PathBuf, sync::Arc};

use atlas_agentic::conversations::{
    Conversation, ConversationAskResult, ConversationId, ConversationMessage, ConversationMetadata,
    ConversationService, ConversationStore, MessageRole,
};
use atlas_agentic::{app::build_runtime, Runtime};
use serde::Serialize;
use tauri::{ipc::Channel, State};

type RuntimeFactory = Arc<dyn Fn() -> Result<Runtime, String> + Send + Sync>;

#[derive(Clone)]
pub struct DesktopState {
    store: ConversationStore,
    history_limit: usize,
    runtime_factory: RuntimeFactory,
}

impl DesktopState {
    #[cfg(test)]
    pub fn new(store: ConversationStore) -> Self {
        Self::with_runtime_factory(
            store,
            40,
            Arc::new(|| Err("Atlas runtime is not configured".to_string())),
        )
    }

    pub fn with_runtime_paths(
        store: ConversationStore,
        history_limit: usize,
        workspace_root: PathBuf,
        agent_file: PathBuf,
    ) -> Self {
        Self::with_runtime_factory(
            store,
            history_limit,
            Arc::new(move || build_runtime(&workspace_root, &agent_file)),
        )
    }

    fn with_runtime_factory(
        store: ConversationStore,
        history_limit: usize,
        runtime_factory: RuntimeFactory,
    ) -> Self {
        Self {
            store,
            history_limit,
            runtime_factory,
        }
    }

    #[cfg(test)]
    fn ask_with_sink(
        &self,
        prompt: &str,
        conversation_id: Option<&ConversationId>,
        on_delta: &mut dyn FnMut(&str),
    ) -> Result<ConversationAskResultDto, String> {
        let runtime = (self.runtime_factory)()?;
        ConversationService::new(&runtime, &self.store, self.history_limit)
            .ask(prompt, conversation_id, on_delta)
            .map(Into::into)
    }

    fn ask_with_fallible_sink(
        &self,
        prompt: &str,
        conversation_id: Option<&ConversationId>,
        on_delta: &mut dyn FnMut(&str) -> Result<(), String>,
    ) -> Result<ConversationAskResultDto, String> {
        let runtime = (self.runtime_factory)()?;
        ConversationService::new(&runtime, &self.store, self.history_limit)
            .ask_with_fallible_sink(prompt, conversation_id, on_delta)
            .map(Into::into)
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConversationAskResultDto {
    pub answer: String,
    pub metadata: ConversationMetadataDto,
    pub created: bool,
}

impl From<ConversationAskResult> for ConversationAskResultDto {
    fn from(result: ConversationAskResult) -> Self {
        Self {
            answer: result.answer,
            metadata: result.metadata.into(),
            created: result.created,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "event", content = "data", rename_all = "camelCase")]
pub enum ChatEvent {
    Delta { content: String },
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

async fn ask_in_background<F>(
    prompt: String,
    conversation_id: Option<String>,
    state: DesktopState,
    mut on_delta: F,
) -> Result<ConversationAskResultDto, String>
where
    F: FnMut(&str) -> Result<(), String> + Send + 'static,
{
    tauri::async_runtime::spawn_blocking(move || {
        let conversation_id = conversation_id.as_deref().map(parse_id).transpose()?;
        state.ask_with_fallible_sink(&prompt, conversation_id.as_ref(), &mut on_delta)
    })
    .await
    .map_err(|error| format!("Atlas ask worker failed: {error}"))?
}

#[tauri::command]
pub async fn ask(
    prompt: String,
    conversation_id: Option<String>,
    channel: Channel<ChatEvent>,
    state: State<'_, DesktopState>,
) -> Result<ConversationAskResultDto, String> {
    ask_in_background(
        prompt,
        conversation_id,
        state.inner().clone(),
        move |delta| {
            channel
                .send(ChatEvent::Delta {
                    content: delta.to_string(),
                })
                .map_err(|error| format!("failed to stream response: {error}"))
        },
    )
    .await
}

#[cfg(test)]
mod tests {
    use std::{
        collections::VecDeque,
        sync::{Arc, Mutex},
    };

    use atlas_agentic::{
        agents::{Agent, AgentRegistry, LocalAgent, LocalAgentDefinition},
        conversations::{ConversationMessage, ConversationStore},
        models::deepseek::{DeepSeekConfig, DeepSeekProvider},
        tools::ToolDefinition,
        Runtime,
    };
    use tempfile::TempDir;

    use super::{ask_in_background, delete_impl, list_impl, load_impl, rename_impl, DesktopState};

    #[derive(Clone)]
    struct FakeResponse {
        chunks: Vec<&'static str>,
        error: Option<&'static str>,
    }

    struct FakeAgent {
        responses: Arc<Mutex<VecDeque<FakeResponse>>>,
        histories: Arc<Mutex<Vec<Vec<ConversationMessage>>>>,
    }

    impl Agent for FakeAgent {
        fn name(&self) -> &str {
            "fake"
        }

        fn system_prompt(&self) -> &str {
            "fake"
        }

        fn process(
            &self,
            _input: &str,
            _history: &[ConversationMessage],
            _tools: &[ToolDefinition],
        ) -> Result<String, String> {
            Err("fake agent requires streaming".to_string())
        }

        fn process_stream(
            &self,
            _input: &str,
            history: &[ConversationMessage],
            _tools: &[ToolDefinition],
            on_delta: &mut dyn FnMut(&str),
        ) -> Result<String, String> {
            self.histories.lock().unwrap().push(history.to_vec());
            let response = self
                .responses
                .lock()
                .unwrap()
                .pop_front()
                .expect("fake response queue exhausted");
            let mut answer = String::new();
            for chunk in response.chunks {
                on_delta(chunk);
                answer.push_str(chunk);
            }
            if let Some(error) = response.error {
                Err(error.to_string())
            } else {
                Ok(answer)
            }
        }

        fn generate_conversation_title(&self, _first_input: &str) -> Result<String, String> {
            Ok("Desktop title".to_string())
        }
    }

    fn state_with_responses(
        store: ConversationStore,
        responses: Vec<FakeResponse>,
        histories: Arc<Mutex<Vec<Vec<ConversationMessage>>>>,
    ) -> DesktopState {
        let responses = Arc::new(Mutex::new(responses.into_iter().collect()));
        let runtime_factory = Arc::new(move || {
            Ok(Runtime::new(AgentRegistry::new(vec![Box::new(
                FakeAgent {
                    responses: Arc::clone(&responses),
                    histories: Arc::clone(&histories),
                },
            )])))
        });
        DesktopState::with_runtime_factory(store, 40, runtime_factory)
    }

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

    #[test]
    fn ask_forwards_deltas_in_order_and_returns_metadata() {
        let data = TempDir::new().unwrap();
        let store = ConversationStore::at(data.path().to_path_buf());
        let histories = Arc::new(Mutex::new(Vec::new()));
        let state = state_with_responses(
            store.clone(),
            vec![FakeResponse {
                chunks: vec!["Hello", " ", "desktop"],
                error: None,
            }],
            histories,
        );
        let mut deltas = Vec::new();

        let result = state
            .ask_with_sink("hello", None, &mut |delta| deltas.push(delta.to_string()))
            .unwrap();

        assert_eq!(deltas, vec!["Hello", " ", "desktop"]);
        assert_eq!(result.answer, "Hello desktop");
        assert!(result.created);
        assert_eq!(result.metadata.title, "Desktop title");
        assert_eq!(store.list().unwrap().len(), 1);
    }

    #[test]
    fn first_ask_failure_leaves_store_empty() {
        let data = TempDir::new().unwrap();
        let store = ConversationStore::at(data.path().to_path_buf());
        let histories = Arc::new(Mutex::new(Vec::new()));
        let state = state_with_responses(
            store.clone(),
            vec![FakeResponse {
                chunks: vec!["partial"],
                error: Some("model failed"),
            }],
            histories,
        );
        let mut deltas = Vec::new();

        let error = state
            .ask_with_sink("hello", None, &mut |delta| deltas.push(delta.to_string()))
            .unwrap_err();

        assert_eq!(error, "model failed");
        assert_eq!(deltas, vec!["partial"]);
        assert!(store.list().unwrap().is_empty());
    }

    #[test]
    fn continuing_ask_passes_history_and_keeps_id() {
        let data = TempDir::new().unwrap();
        let store = ConversationStore::at(data.path().to_path_buf());
        let saved = store.create_turn("first question", "first answer").unwrap();
        let histories = Arc::new(Mutex::new(Vec::new()));
        let state = state_with_responses(
            store.clone(),
            vec![FakeResponse {
                chunks: vec!["next answer"],
                error: None,
            }],
            Arc::clone(&histories),
        );

        let result = state
            .ask_with_sink("next question", Some(&saved.id), &mut |_| {})
            .unwrap();

        assert_eq!(result.metadata.id, saved.id.to_string());
        assert!(!result.created);
        let captured = histories.lock().unwrap();
        assert_eq!(captured.len(), 1);
        assert_eq!(captured[0].len(), 2);
        assert_eq!(captured[0][0].content, "first question");
        assert_eq!(captured[0][1].content, "first answer");
        assert_eq!(store.load(&saved.id).unwrap().messages.len(), 4);
    }

    #[test]
    fn provider_failure_does_not_damage_history() {
        let data = TempDir::new().unwrap();
        let store = ConversationStore::at(data.path().to_path_buf());
        let saved = store.create_turn("first question", "first answer").unwrap();
        let state = DesktopState::with_runtime_factory(
            store.clone(),
            40,
            Arc::new(|| Err("provider is not configured".to_string())),
        );

        let error = state
            .ask_with_sink("next question", Some(&saved.id), &mut |_| {})
            .unwrap_err();

        assert_eq!(error, "provider is not configured");
        let loaded = store.load(&saved.id).unwrap();
        assert_eq!(loaded.messages.len(), 2);
        assert_eq!(loaded.metadata.message_count, 2);
    }

    #[test]
    fn configured_ask_constructs_the_blocking_provider_off_the_async_executor() {
        let data = TempDir::new().unwrap();
        let store = ConversationStore::at(data.path().to_path_buf());
        let runtime_factory = Arc::new(|| {
            let definition = LocalAgentDefinition::from_toml(
                r#"
name = "atlas"
display_name = "Atlas"
model_provider = "deepseek"
model = "deepseek-flash"
system_prompt = "You are Atlas."
conversation_title_prompt = "Return a short title."
"#,
            )?;
            let provider = Box::new(DeepSeekProvider::new(DeepSeekConfig {
                api_key: "test-key".to_string(),
                base_url: "http://127.0.0.1:9".to_string(),
                model: "deepseek-flash".to_string(),
            }));
            Ok(Runtime::new(AgentRegistry::new(vec![Box::new(
                LocalAgent::new(definition, provider),
            )])))
        });
        let state = DesktopState::with_runtime_factory(store.clone(), 40, runtime_factory);

        let error = tauri::async_runtime::block_on(ask_in_background(
            "hello".to_string(),
            None,
            state,
            |_| Ok(()),
        ))
        .unwrap_err();

        assert!(error.contains("DeepSeek request failed"), "{error}");
        assert!(store.list().unwrap().is_empty());
    }
}
