use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

use atlas_agentic::conversations::{
    validate_workspace as validate_workspace_path, Conversation, ConversationAskResult,
    ConversationId, ConversationMessage, ConversationMetadata, ConversationService,
    ConversationStore, MessageRole,
};
use atlas_agentic::{
    app::{build_runtime_with_settings, list_available_models_with_settings},
    models::ModelInfo,
    settings::{AtlasSettings, EffectiveSettings, SettingField, SettingsStore},
    workspace::automatic_workspace,
    Runtime,
};
use serde::{Deserialize, Serialize};
use tauri::{ipc::Channel, State};

type RuntimeFactory =
    Arc<dyn Fn(&Path, &EffectiveSettings) -> Result<Runtime, String> + Send + Sync>;
type AutomaticWorkspaceResolver = Arc<dyn Fn() -> Result<PathBuf, String> + Send + Sync>;

#[derive(Clone)]
pub struct DesktopState {
    store: ConversationStore,
    settings_store: SettingsStore,
    agent_file: PathBuf,
    runtime_factory: RuntimeFactory,
    automatic_workspace_resolver: AutomaticWorkspaceResolver,
}

impl DesktopState {
    #[cfg(test)]
    pub fn new(store: ConversationStore) -> Self {
        let settings_store = test_settings_store();
        Self::new_with_settings(store, settings_store)
    }

    #[cfg(test)]
    pub fn new_with_settings(store: ConversationStore, settings_store: SettingsStore) -> Self {
        Self::with_runtime_factory_settings_and_workspace_resolver(
            store,
            settings_store,
            PathBuf::from("agents/atlas.toml"),
            Arc::new(|_, _| Err("Atlas runtime is not configured".to_string())),
            test_automatic_workspace_resolver(),
        )
    }

    pub fn with_runtime_paths(
        store: ConversationStore,
        settings_store: SettingsStore,
        agent_file: PathBuf,
    ) -> Self {
        let runtime_agent_file = agent_file.clone();
        Self::with_runtime_factory_and_settings(
            store,
            settings_store,
            agent_file,
            Arc::new(move |workspace, settings| {
                build_runtime_with_settings(workspace, &runtime_agent_file, settings)
            }),
        )
    }

    #[cfg(test)]
    fn with_runtime_factory(
        store: ConversationStore,
        _history_limit: usize,
        runtime_factory: Arc<dyn Fn() -> Result<Runtime, String> + Send + Sync>,
    ) -> Self {
        let settings_store = test_settings_store();
        Self::with_runtime_factory_settings_and_workspace_resolver(
            store,
            settings_store,
            PathBuf::from("agents/atlas.toml"),
            Arc::new(move |_, _| runtime_factory()),
            test_automatic_workspace_resolver(),
        )
    }

    fn with_runtime_factory_and_settings(
        store: ConversationStore,
        settings_store: SettingsStore,
        agent_file: PathBuf,
        runtime_factory: RuntimeFactory,
    ) -> Self {
        Self::with_runtime_factory_settings_and_workspace_resolver(
            store,
            settings_store,
            agent_file,
            runtime_factory,
            Arc::new(automatic_workspace),
        )
    }

    fn with_runtime_factory_settings_and_workspace_resolver(
        store: ConversationStore,
        settings_store: SettingsStore,
        agent_file: PathBuf,
        runtime_factory: RuntimeFactory,
        automatic_workspace_resolver: AutomaticWorkspaceResolver,
    ) -> Self {
        Self {
            store,
            settings_store,
            agent_file,
            runtime_factory,
            automatic_workspace_resolver,
        }
    }

    #[cfg(test)]
    fn ask_with_sink(
        &self,
        prompt: &str,
        conversation_id: Option<&ConversationId>,
        workspace_path: Option<&Path>,
        on_delta: &mut dyn FnMut(&str),
    ) -> Result<ConversationAskResultDto, String> {
        self.ask_with_workspace_and_fallible_sink(
            prompt,
            conversation_id,
            workspace_path,
            &mut |delta| {
                on_delta(delta);
                Ok(())
            },
        )
    }

    fn ask_with_workspace_and_fallible_sink(
        &self,
        prompt: &str,
        conversation_id: Option<&ConversationId>,
        workspace_path: Option<&Path>,
        on_delta: &mut dyn FnMut(&str) -> Result<(), String>,
    ) -> Result<ConversationAskResultDto, String> {
        let effective = self.settings_store.load()?.resolve_from_env()?;
        let workspace = self.resolve_workspace(conversation_id, workspace_path, &effective)?;
        let runtime = (self.runtime_factory)(&workspace, &effective)?;
        ConversationService::new(&runtime, &self.store, effective.history_limit)
            .ask_with_workspace_and_fallible_sink(
                prompt,
                conversation_id,
                Some(workspace.as_path()),
                on_delta,
            )
            .map(Into::into)
    }

    fn resolve_workspace(
        &self,
        conversation_id: Option<&ConversationId>,
        requested: Option<&Path>,
        effective: &EffectiveSettings,
    ) -> Result<PathBuf, String> {
        match conversation_id {
            Some(id) => match self.store.load(id)?.metadata.workspace_path {
                Some(saved) => validate_workspace_path(&saved),
                None => self
                    .resolve_default_workspace(effective)
                    .map(|(workspace, _)| workspace),
            },
            None => match requested {
                Some(selected) => validate_workspace_path(selected),
                None => self
                    .resolve_default_workspace(effective)
                    .map(|(workspace, _)| workspace),
            },
        }
    }

    fn resolve_default_workspace(
        &self,
        effective: &EffectiveSettings,
    ) -> Result<(PathBuf, bool), String> {
        match effective.default_workspace.as_deref() {
            Some(configured) => validate_workspace_path(configured).map(|path| (path, false)),
            None => {
                let automatic = (self.automatic_workspace_resolver)()?;
                validate_workspace_path(&automatic).map(|path| (path, true))
            }
        }
    }
}

#[cfg(test)]
fn test_settings_store() -> SettingsStore {
    SettingsStore::at(
        std::env::temp_dir()
            .join(format!("atlas-desktop-test-{}", ConversationId::new()))
            .join("settings.json"),
    )
}

#[cfg(test)]
fn test_automatic_workspace_resolver() -> AutomaticWorkspaceResolver {
    let path = std::env::temp_dir().join(format!(
        "atlas-desktop-test-workspace-{}",
        ConversationId::new()
    ));
    std::fs::create_dir_all(&path).expect("test automatic workspace must be creatable");
    Arc::new(move || Ok(path.clone()))
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceDto {
    pub path: String,
    pub name: String,
}

fn workspace_dto(path: &Path) -> WorkspaceDto {
    WorkspaceDto {
        path: path.display().to_string(),
        name: path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or_else(|| path.to_str().unwrap_or("Workspace"))
            .to_string(),
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
    pub workspace: Option<WorkspaceDto>,
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
            workspace: metadata.workspace_path.as_deref().map(workspace_dto),
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SettingsViewDto {
    pub api_key_source: &'static str,
    pub base_url: String,
    pub model: String,
    pub active_model: String,
    pub history_limit: usize,
    pub default_workspace: Option<WorkspaceDto>,
    pub default_workspace_automatic: bool,
    pub default_workspace_error: Option<String>,
    pub environment_overrides: Vec<&'static str>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SaveSettingsInput {
    pub api_key: Option<String>,
    pub clear_api_key: bool,
    pub base_url: String,
    pub model: String,
    pub history_limit: usize,
    pub default_workspace_path: Option<String>,
}

fn load_settings_impl(state: &DesktopState) -> Result<SettingsViewDto, String> {
    let saved = state.settings_store.load()?;
    let effective = saved.resolve_from_env()?;
    let (default_workspace, default_workspace_automatic, default_workspace_error) =
        match effective.default_workspace.as_deref() {
            Some(configured) => match validate_workspace_path(configured) {
                Ok(workspace) => (Some(workspace_dto(&workspace)), false, None),
                Err(error) => (Some(workspace_dto(configured)), false, Some(error)),
            },
            None => match (state.automatic_workspace_resolver)() {
                Ok(workspace) => match validate_workspace_path(&workspace) {
                    Ok(workspace) => (Some(workspace_dto(&workspace)), true, None),
                    Err(error) => (None, true, Some(error)),
                },
                Err(error) => (None, true, Some(error)),
            },
        };
    Ok(settings_view(
        &saved,
        effective,
        default_workspace,
        default_workspace_automatic,
        default_workspace_error,
    ))
}

fn settings_view(
    saved: &AtlasSettings,
    effective: EffectiveSettings,
    default_workspace: Option<WorkspaceDto>,
    default_workspace_automatic: bool,
    default_workspace_error: Option<String>,
) -> SettingsViewDto {
    let api_key_source = if effective
        .environment_overrides
        .contains(&SettingField::ApiKey)
    {
        "environment"
    } else if saved.deepseek.api_key.is_some() {
        "settings"
    } else {
        "missing"
    };
    let environment_overrides = effective
        .environment_overrides
        .iter()
        .map(|field| match field {
            SettingField::ApiKey => "apiKey",
            SettingField::BaseUrl => "baseUrl",
            SettingField::Model => "model",
            SettingField::HistoryLimit => "historyLimit",
        })
        .collect();
    SettingsViewDto {
        api_key_source,
        base_url: saved.deepseek.base_url.clone(),
        model: saved.deepseek.model.clone(),
        active_model: effective.model.clone(),
        history_limit: saved.conversation.history_limit,
        default_workspace,
        default_workspace_automatic,
        default_workspace_error,
        environment_overrides,
    }
}

fn save_settings_impl(
    input: SaveSettingsInput,
    state: &DesktopState,
) -> Result<SettingsViewDto, String> {
    if input.clear_api_key && input.api_key.is_some() {
        return Err("cannot set and clear the API key in the same save".to_string());
    }
    state.settings_store.update(|saved| {
        if input.clear_api_key {
            saved.deepseek.api_key = None;
        } else if let Some(api_key) = input.api_key {
            if api_key.trim().is_empty() {
                return Err("DeepSeek API key cannot be blank".to_string());
            }
            saved.deepseek.api_key = Some(api_key);
        }
        saved.deepseek.base_url = input.base_url;
        saved.deepseek.model = input.model;
        saved.conversation.history_limit = input.history_limit;
        saved.workspace.default_path = input
            .default_workspace_path
            .as_deref()
            .map(Path::new)
            .map(validate_workspace_path)
            .transpose()?;
        Ok(())
    })?;
    load_settings_impl(state)
}

fn list_models_impl(state: &DesktopState) -> Result<Vec<ModelInfo>, String> {
    let settings = state.settings_store.load()?.resolve_from_env()?;
    list_available_models_with_settings(&state.agent_file, &settings)
}

async fn list_models_in_background(state: DesktopState) -> Result<Vec<ModelInfo>, String> {
    tauri::async_runtime::spawn_blocking(move || list_models_impl(&state))
        .await
        .map_err(|error| format!("Atlas model list worker failed: {error}"))?
}

fn set_model_impl(model: &str, state: &DesktopState) -> Result<SettingsViewDto, String> {
    let model = model.trim();
    if model.is_empty() {
        return Err("DeepSeek model cannot be blank".to_string());
    }
    state.settings_store.update(|saved| {
        let effective = saved.resolve_from_env()?;
        if effective
            .environment_overrides
            .contains(&SettingField::Model)
        {
            return Err("model is controlled by DEEPSEEK_MODEL".to_string());
        }
        saved.deepseek.model = model.to_string();
        Ok(())
    })?;
    load_settings_impl(state)
}

fn validate_workspace_impl(path: &str) -> Result<WorkspaceDto, String> {
    validate_workspace_path(Path::new(path)).map(|path| workspace_dto(&path))
}

fn assign_workspace_impl(
    id: &str,
    path: &str,
    state: &DesktopState,
) -> Result<ConversationMetadataDto, String> {
    state
        .store
        .assign_workspace(&parse_id(id)?, Path::new(path))
        .map(Into::into)
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

#[tauri::command]
pub fn load_settings(state: State<'_, DesktopState>) -> Result<SettingsViewDto, String> {
    load_settings_impl(&state)
}

#[tauri::command]
pub fn save_settings(
    input: SaveSettingsInput,
    state: State<'_, DesktopState>,
) -> Result<SettingsViewDto, String> {
    save_settings_impl(input, &state)
}

#[tauri::command]
pub async fn list_models(state: State<'_, DesktopState>) -> Result<Vec<ModelInfo>, String> {
    list_models_in_background(state.inner().clone()).await
}

#[tauri::command]
pub fn set_model(model: String, state: State<'_, DesktopState>) -> Result<SettingsViewDto, String> {
    set_model_impl(&model, &state)
}

#[tauri::command]
pub fn validate_workspace(path: String) -> Result<WorkspaceDto, String> {
    validate_workspace_impl(&path)
}

#[tauri::command]
pub fn assign_conversation_workspace(
    id: String,
    path: String,
    state: State<'_, DesktopState>,
) -> Result<ConversationMetadataDto, String> {
    assign_workspace_impl(&id, &path, &state)
}

async fn ask_in_background<F>(
    prompt: String,
    conversation_id: Option<String>,
    workspace_path: Option<String>,
    state: DesktopState,
    mut on_delta: F,
) -> Result<ConversationAskResultDto, String>
where
    F: FnMut(&str) -> Result<(), String> + Send + 'static,
{
    tauri::async_runtime::spawn_blocking(move || {
        let conversation_id = conversation_id.as_deref().map(parse_id).transpose()?;
        state.ask_with_workspace_and_fallible_sink(
            &prompt,
            conversation_id.as_ref(),
            workspace_path.as_deref().map(Path::new),
            &mut on_delta,
        )
    })
    .await
    .map_err(|error| format!("Atlas ask worker failed: {error}"))?
}

#[tauri::command]
pub async fn ask(
    prompt: String,
    conversation_id: Option<String>,
    workspace_path: Option<String>,
    channel: Channel<ChatEvent>,
    state: State<'_, DesktopState>,
) -> Result<ConversationAskResultDto, String> {
    ask_in_background(
        prompt,
        conversation_id,
        workspace_path,
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
        ffi::OsString,
        io::{Read, Write},
        net::TcpListener,
        path::{Path, PathBuf},
        sync::{
            atomic::{AtomicUsize, Ordering},
            Arc, Mutex, MutexGuard, OnceLock,
        },
    };

    use atlas_agentic::{
        agents::{Agent, AgentRegistry, LocalAgent, LocalAgentDefinition},
        conversations::{ConversationId, ConversationMessage, ConversationStore},
        models::deepseek::{DeepSeekConfig, DeepSeekProvider},
        settings::{AtlasSettings, SettingsStore},
        tools::ToolDefinition,
        Runtime,
    };
    use tempfile::TempDir;

    use super::{
        ask_in_background, delete_impl, list_impl, list_models_in_background, load_impl,
        load_settings_impl, rename_impl, save_settings_impl, set_model_impl, settings_view,
        validate_workspace_impl, DesktopState, SaveSettingsInput,
    };

    fn environment_lock() -> MutexGuard<'static, ()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(())).lock().unwrap()
    }

    struct EnvironmentRestore(Vec<(&'static str, Option<OsString>)>);

    impl EnvironmentRestore {
        fn clear(names: &[&'static str]) -> Self {
            Self(
                names
                    .iter()
                    .map(|name| {
                        let previous = std::env::var_os(name);
                        std::env::remove_var(name);
                        (*name, previous)
                    })
                    .collect(),
            )
        }
    }

    impl Drop for EnvironmentRestore {
        fn drop(&mut self) {
            for (name, previous) in self.0.drain(..) {
                match previous {
                    Some(value) => std::env::set_var(name, value),
                    None => std::env::remove_var(name),
                }
            }
        }
    }

    #[test]
    fn model_list_uses_saved_provider_configuration() {
        let _environment = environment_lock();
        let _restore =
            EnvironmentRestore::clear(&["DEEPSEEK_API_KEY", "DEEPSEEK_BASE_URL", "DEEPSEEK_MODEL"]);
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0_u8; 4096];
            let read = stream.read(&mut request).unwrap();
            let request = String::from_utf8_lossy(&request[..read]);
            assert!(request.starts_with("GET /models HTTP/1.1\r\n"), "{request}");
            let body = r#"{"object":"list","data":[{"id":"deepseek-flash","object":"model","owned_by":"deepseek","name":"DeepSeek Flash"}]}"#;
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            )
            .unwrap();
        });
        let root = TempDir::new().unwrap();
        let agent_file = root.path().join("atlas.toml");
        std::fs::write(&agent_file, include_str!("../../../agents/atlas.toml")).unwrap();
        let settings_store = SettingsStore::at(root.path().join("settings.json"));
        let mut settings = AtlasSettings::default();
        settings.deepseek.api_key = Some("saved-key".to_string());
        settings.deepseek.base_url = format!("http://{address}");
        settings_store.save(&settings).unwrap();
        let state = DesktopState::with_runtime_paths(
            ConversationStore::at(root.path().join("data")),
            settings_store,
            agent_file,
        );

        let models = tauri::async_runtime::block_on(list_models_in_background(state)).unwrap();

        server.join().unwrap();
        assert_eq!(models[0].id, "deepseek-flash");
        assert_eq!(models[0].name, "DeepSeek Flash");
    }

    #[test]
    fn model_selection_updates_only_the_saved_model() {
        let _environment = environment_lock();
        let _restore =
            EnvironmentRestore::clear(&["DEEPSEEK_API_KEY", "DEEPSEEK_BASE_URL", "DEEPSEEK_MODEL"]);
        let root = TempDir::new().unwrap();
        let settings_store = SettingsStore::at(root.path().join("settings.json"));
        let mut saved = AtlasSettings::default();
        saved.deepseek.api_key = Some("saved-key".to_string());
        saved.workspace.default_path = Some(root.path().join("stale-workspace"));
        settings_store.save(&saved).unwrap();
        let state = DesktopState::new_with_settings(
            ConversationStore::at(root.path().join("data")),
            settings_store.clone(),
        );

        let view = set_model_impl("deepseek-v4-pro", &state).unwrap();

        let updated = settings_store.load().unwrap();
        assert_eq!(view.model, "deepseek-v4-pro");
        assert_eq!(updated.deepseek.model, "deepseek-v4-pro");
        assert_eq!(updated.deepseek.api_key.as_deref(), Some("saved-key"));
        assert_eq!(updated.workspace.default_path, saved.workspace.default_path);
    }

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

    fn state_with_responses_and_automatic(
        store: ConversationStore,
        settings_store: SettingsStore,
        automatic: PathBuf,
        responses: Vec<FakeResponse>,
        runtime_workspaces: Arc<Mutex<Vec<PathBuf>>>,
    ) -> DesktopState {
        let responses = Arc::new(Mutex::new(responses.into_iter().collect()));
        DesktopState::with_runtime_factory_settings_and_workspace_resolver(
            store,
            settings_store,
            PathBuf::from("agents/atlas.toml"),
            {
                let runtime_workspaces = Arc::clone(&runtime_workspaces);
                Arc::new(move |workspace, _| {
                    runtime_workspaces
                        .lock()
                        .unwrap()
                        .push(workspace.to_path_buf());
                    Ok(Runtime::new(AgentRegistry::new(vec![Box::new(
                        FakeAgent {
                            responses: Arc::clone(&responses),
                            histories: Arc::new(Mutex::new(Vec::new())),
                        },
                    )])))
                })
            },
            Arc::new(move || Ok(automatic.clone())),
        )
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
            .ask_with_sink("hello", None, Some(data.path()), &mut |delta| {
                deltas.push(delta.to_string())
            })
            .unwrap();

        assert_eq!(deltas, vec!["Hello", " ", "desktop"]);
        assert_eq!(result.answer, "Hello desktop");
        assert!(result.created);
        assert_eq!(result.metadata.title, "Desktop title");
        assert_eq!(store.list().unwrap().len(), 1);
    }

    #[test]
    fn automatic_workspace_is_the_fallback_for_new_and_legacy_conversations() {
        let root = TempDir::new().unwrap();
        let automatic = root.path().join("Atlas");
        std::fs::create_dir(&automatic).unwrap();
        let store = ConversationStore::at(root.path().join("data"));
        let settings_store = SettingsStore::at(root.path().join("settings.json"));
        let legacy = store.create_turn("old question", "old answer").unwrap();
        let state = state_with_responses_and_automatic(
            store.clone(),
            settings_store,
            automatic.clone(),
            vec![],
            Arc::new(Mutex::new(Vec::new())),
        );
        let effective = AtlasSettings::default().resolve_with(|_| None).unwrap();

        assert_eq!(
            state.resolve_workspace(None, None, &effective).unwrap(),
            automatic.canonicalize().unwrap()
        );
        assert_eq!(
            state
                .resolve_workspace(Some(&legacy.id), None, &effective)
                .unwrap(),
            automatic.canonicalize().unwrap()
        );
    }

    #[test]
    fn requested_configured_and_saved_workspaces_follow_authority_order() {
        let root = TempDir::new().unwrap();
        let automatic = root.path().join("Atlas");
        let configured = root.path().join("configured");
        let requested = root.path().join("requested");
        let saved_workspace = root.path().join("saved");
        for path in [&automatic, &configured, &requested, &saved_workspace] {
            std::fs::create_dir(path).unwrap();
        }
        let store = ConversationStore::at(root.path().join("data"));
        let saved = store
            .create_turn_with_workspace("old question", "old answer", Some(&saved_workspace))
            .unwrap();
        let settings_store = SettingsStore::at(root.path().join("settings.json"));
        let state = state_with_responses_and_automatic(
            store,
            settings_store,
            automatic,
            vec![],
            Arc::new(Mutex::new(Vec::new())),
        );
        let mut settings = AtlasSettings::default();
        settings.workspace.default_path = Some(configured.clone());
        let effective = settings.resolve_with(|_| None).unwrap();

        assert_eq!(
            state
                .resolve_workspace(None, Some(&requested), &effective)
                .unwrap(),
            requested.canonicalize().unwrap()
        );
        assert_eq!(
            state.resolve_workspace(None, None, &effective).unwrap(),
            configured.canonicalize().unwrap()
        );
        assert_eq!(
            state
                .resolve_workspace(Some(&saved.id), Some(&requested), &effective)
                .unwrap(),
            saved_workspace.canonicalize().unwrap()
        );
    }

    #[test]
    fn successful_legacy_ask_uses_and_assigns_the_automatic_workspace() {
        let root = TempDir::new().unwrap();
        let automatic = root.path().join("Atlas");
        std::fs::create_dir(&automatic).unwrap();
        let store = ConversationStore::at(root.path().join("data"));
        let legacy = store.create_turn("old question", "old answer").unwrap();
        let runtime_workspaces = Arc::new(Mutex::new(Vec::new()));
        let state = state_with_responses_and_automatic(
            store.clone(),
            SettingsStore::at(root.path().join("settings.json")),
            automatic.clone(),
            vec![FakeResponse {
                chunks: vec!["next answer"],
                error: None,
            }],
            Arc::clone(&runtime_workspaces),
        );

        let result = state
            .ask_with_sink("next question", Some(&legacy.id), None, &mut |_| {})
            .unwrap();

        let expected = automatic.canonicalize().unwrap();
        assert_eq!(
            result.metadata.workspace.unwrap().path,
            expected.display().to_string()
        );
        assert_eq!(*runtime_workspaces.lock().unwrap(), vec![expected.clone()]);
        assert_eq!(
            store.load(&legacy.id).unwrap().metadata.workspace_path,
            Some(expected)
        );
    }

    #[test]
    fn automatic_settings_view_is_marked_and_configured_default_stays_explicit() {
        let root = TempDir::new().unwrap();
        let automatic = root.path().join("Atlas");
        let configured = root.path().join("configured");
        std::fs::create_dir(&automatic).unwrap();
        std::fs::create_dir(&configured).unwrap();
        let settings_store = SettingsStore::at(root.path().join("settings.json"));
        let state = state_with_responses_and_automatic(
            ConversationStore::at(root.path().join("data")),
            settings_store.clone(),
            automatic.clone(),
            vec![],
            Arc::new(Mutex::new(Vec::new())),
        );

        let automatic_view = load_settings_impl(&state).unwrap();
        assert!(automatic_view.default_workspace_automatic);
        assert_eq!(automatic_view.default_workspace_error, None);
        assert_eq!(
            automatic_view.default_workspace.unwrap().path,
            automatic.canonicalize().unwrap().display().to_string()
        );

        let mut settings = AtlasSettings::default();
        settings.workspace.default_path = Some(configured.clone());
        settings_store.save(&settings).unwrap();
        let configured_view = load_settings_impl(&state).unwrap();
        assert!(!configured_view.default_workspace_automatic);
        assert_eq!(configured_view.default_workspace_error, None);
        assert_eq!(
            configured_view.default_workspace.unwrap().path,
            configured.canonicalize().unwrap().display().to_string()
        );
    }

    #[test]
    fn settings_remain_editable_when_the_configured_default_is_stale() {
        let root = TempDir::new().unwrap();
        let stale = root.path().join("deleted-default");
        let settings_store = SettingsStore::at(root.path().join("settings.json"));
        let mut settings = AtlasSettings::default();
        settings.workspace.default_path = Some(stale.clone());
        settings_store.save(&settings).unwrap();
        let automatic_calls = Arc::new(AtomicUsize::new(0));
        let state = DesktopState::with_runtime_factory_settings_and_workspace_resolver(
            ConversationStore::at(root.path().join("data")),
            settings_store,
            PathBuf::from("agents/atlas.toml"),
            Arc::new(|_, _| Err("runtime must not be built".to_string())),
            {
                let automatic_calls = Arc::clone(&automatic_calls);
                Arc::new(move || {
                    automatic_calls.fetch_add(1, Ordering::SeqCst);
                    Err("automatic resolver must not run".to_string())
                })
            },
        );

        let view = load_settings_impl(&state).unwrap();

        assert!(!view.default_workspace_automatic);
        assert_eq!(
            view.default_workspace.unwrap().path,
            stale.display().to_string()
        );
        assert!(view.default_workspace_error.unwrap().contains("workspace"));
        assert_eq!(automatic_calls.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn settings_remain_editable_when_the_automatic_workspace_is_unavailable() {
        let root = TempDir::new().unwrap();
        let state = DesktopState::with_runtime_factory_settings_and_workspace_resolver(
            ConversationStore::at(root.path().join("data")),
            SettingsStore::at(root.path().join("settings.json")),
            PathBuf::from("agents/atlas.toml"),
            Arc::new(|_, _| Err("runtime must not be built".to_string())),
            Arc::new(|| Err("could not resolve the user home directory".to_string())),
        );

        let view = load_settings_impl(&state).unwrap();

        assert!(view.default_workspace_automatic);
        assert_eq!(view.default_workspace, None);
        assert_eq!(
            view.default_workspace_error.as_deref(),
            Some("could not resolve the user home directory")
        );
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
            .ask_with_sink("hello", None, Some(data.path()), &mut |delta| {
                deltas.push(delta.to_string())
            })
            .unwrap_err();

        assert_eq!(error, "model failed");
        assert_eq!(deltas, vec!["partial"]);
        assert!(store.list().unwrap().is_empty());
    }

    #[test]
    fn continuing_ask_passes_history_and_keeps_id() {
        let data = TempDir::new().unwrap();
        let store = ConversationStore::at(data.path().to_path_buf());
        let saved = store
            .create_turn_with_workspace("first question", "first answer", Some(data.path()))
            .unwrap();
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
            .ask_with_sink(
                "next question",
                Some(&saved.id),
                Some(Path::new("/frontend/must/be/ignored")),
                &mut |_| {},
            )
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
        let saved = store
            .create_turn_with_workspace("first question", "first answer", Some(data.path()))
            .unwrap();
        let state = DesktopState::with_runtime_factory(
            store.clone(),
            40,
            Arc::new(|| Err("provider is not configured".to_string())),
        );

        let error = state
            .ask_with_sink("next question", Some(&saved.id), None, &mut |_| {})
            .unwrap_err();

        assert_eq!(error, "provider is not configured");
        let loaded = store.load(&saved.id).unwrap();
        assert_eq!(loaded.messages.len(), 2);
        assert_eq!(loaded.metadata.message_count, 2);
    }

    #[test]
    fn legacy_provider_failure_preserves_unassigned_metadata() {
        let root = TempDir::new().unwrap();
        let automatic = root.path().join("Atlas");
        std::fs::create_dir(&automatic).unwrap();
        let store = ConversationStore::at(root.path().join("data"));
        let legacy = store.create_turn("old question", "old answer").unwrap();
        let before = store.load(&legacy.id).unwrap();
        let state = DesktopState::with_runtime_factory_settings_and_workspace_resolver(
            store.clone(),
            SettingsStore::at(root.path().join("settings.json")),
            PathBuf::from("agents/atlas.toml"),
            Arc::new(|_, _| Err("provider is not configured".to_string())),
            Arc::new(move || Ok(automatic.clone())),
        );

        let error = state
            .ask_with_sink("next question", Some(&legacy.id), None, &mut |_| {})
            .unwrap_err();

        assert_eq!(error, "provider is not configured");
        assert_eq!(store.load(&legacy.id).unwrap(), before);
    }

    #[test]
    fn legacy_sink_failure_preserves_unassigned_metadata() {
        let root = TempDir::new().unwrap();
        let automatic = root.path().join("Atlas");
        std::fs::create_dir(&automatic).unwrap();
        let store = ConversationStore::at(root.path().join("data"));
        let legacy = store.create_turn("old question", "old answer").unwrap();
        let before = store.load(&legacy.id).unwrap();
        let state = state_with_responses_and_automatic(
            store.clone(),
            SettingsStore::at(root.path().join("settings.json")),
            automatic,
            vec![FakeResponse {
                chunks: vec!["next answer"],
                error: None,
            }],
            Arc::new(Mutex::new(Vec::new())),
        );

        let error = state
            .ask_with_workspace_and_fallible_sink(
                "next question",
                Some(&legacy.id),
                None,
                &mut |_| Err("stream receiver closed".to_string()),
            )
            .unwrap_err();

        assert_eq!(error, "stream receiver closed");
        assert_eq!(store.load(&legacy.id).unwrap(), before);
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
            Some(data.path().display().to_string()),
            state,
            |_| Ok(()),
        ))
        .unwrap_err();

        assert!(error.contains("DeepSeek request failed"), "{error}");
        assert!(store.list().unwrap().is_empty());
    }

    #[test]
    fn settings_view_reports_credential_source_without_exposing_the_key() {
        let _guard = environment_lock();
        let previous = std::env::var_os("DEEPSEEK_API_KEY");
        std::env::remove_var("DEEPSEEK_API_KEY");
        let data = TempDir::new().unwrap();
        let config = TempDir::new().unwrap();
        let store = ConversationStore::at(data.path().to_path_buf());
        let settings_store = SettingsStore::at(config.path().join("settings.json"));
        let mut saved = AtlasSettings::default();
        saved.deepseek.api_key = Some("saved-secret".to_string());
        settings_store.save(&saved).unwrap();
        let state = DesktopState::new_with_settings(store, settings_store);

        let view = load_settings_impl(&state).unwrap();
        let json = serde_json::to_string(&view).unwrap();
        if let Some(value) = previous {
            std::env::set_var("DEEPSEEK_API_KEY", value);
        }

        assert_eq!(view.api_key_source, "settings");
        assert!(!json.contains("saved-secret"), "{json}");
    }

    #[test]
    fn settings_view_distinguishes_missing_settings_and_environment_sources() {
        let missing = AtlasSettings::default();
        assert_eq!(
            settings_view(
                &missing,
                missing.resolve_with(|_| None).unwrap(),
                Some(super::workspace_dto(Path::new("/tmp/Atlas"))),
                true,
                None,
            )
            .api_key_source,
            "missing"
        );

        let mut saved = AtlasSettings::default();
        saved.deepseek.api_key = Some("saved-secret".to_string());
        assert_eq!(
            settings_view(
                &saved,
                saved.resolve_with(|_| None).unwrap(),
                Some(super::workspace_dto(Path::new("/tmp/Atlas"))),
                true,
                None,
            )
            .api_key_source,
            "settings"
        );

        let environment = saved
            .resolve_with(|name| {
                (name == "DEEPSEEK_API_KEY").then(|| OsString::from("environment-secret"))
            })
            .unwrap();
        let view = settings_view(
            &saved,
            environment,
            Some(super::workspace_dto(Path::new("/tmp/Atlas"))),
            true,
            None,
        );
        let json = serde_json::to_string(&view).unwrap();
        assert_eq!(view.api_key_source, "environment");
        assert_eq!(view.environment_overrides, vec!["apiKey"]);
        assert!(!json.contains("saved-secret"), "{json}");
        assert!(!json.contains("environment-secret"), "{json}");
    }

    #[test]
    fn settings_view_keeps_saved_values_editable_when_environment_overrides_runtime() {
        let mut saved = AtlasSettings::default();
        saved.deepseek.base_url = "https://saved.example.test".to_string();
        saved.deepseek.model = "saved-model".to_string();
        saved.conversation.history_limit = 17;
        let effective = saved
            .resolve_with(|name| match name {
                "DEEPSEEK_BASE_URL" => Some(OsString::from("https://environment.example.test")),
                "DEEPSEEK_MODEL" => Some(OsString::from("environment-model")),
                "ATLAS_HISTORY_MESSAGES" => Some(OsString::from("9")),
                _ => None,
            })
            .unwrap();

        let view = settings_view(
            &saved,
            effective,
            Some(super::workspace_dto(Path::new("/tmp/Atlas"))),
            true,
            None,
        );

        assert_eq!(view.base_url, "https://saved.example.test");
        assert_eq!(view.model, "saved-model");
        assert_eq!(view.active_model, "environment-model");
        assert_eq!(view.history_limit, 17);
        assert_eq!(
            view.environment_overrides,
            vec!["baseUrl", "model", "historyLimit"]
        );
    }

    #[test]
    fn blank_api_key_input_preserves_saved_key_and_clear_removes_only_it() {
        let data = TempDir::new().unwrap();
        let config = TempDir::new().unwrap();
        let store = ConversationStore::at(data.path().to_path_buf());
        let settings_store = SettingsStore::at(config.path().join("settings.json"));
        let mut saved = AtlasSettings::default();
        saved.deepseek.api_key = Some("saved-secret".to_string());
        saved.deepseek.model = "saved-model".to_string();
        settings_store.save(&saved).unwrap();
        let state = DesktopState::new_with_settings(store, settings_store.clone());
        let input = SaveSettingsInput {
            api_key: None,
            clear_api_key: false,
            base_url: saved.deepseek.base_url.clone(),
            model: saved.deepseek.model.clone(),
            history_limit: saved.conversation.history_limit,
            default_workspace_path: None,
        };

        save_settings_impl(input.clone(), &state).unwrap();
        assert_eq!(
            settings_store.load().unwrap().deepseek.api_key.as_deref(),
            Some("saved-secret")
        );
        save_settings_impl(
            SaveSettingsInput {
                clear_api_key: true,
                ..input
            },
            &state,
        )
        .unwrap();
        let cleared = settings_store.load().unwrap();
        assert_eq!(cleared.deepseek.api_key, None);
        assert_eq!(cleared.deepseek.model, "saved-model");
    }

    #[test]
    fn blank_api_key_input_never_copies_an_environment_key_into_settings() {
        let _guard = environment_lock();
        let previous = std::env::var_os("DEEPSEEK_API_KEY");
        std::env::set_var("DEEPSEEK_API_KEY", "environment-secret");
        let data = TempDir::new().unwrap();
        let config = TempDir::new().unwrap();
        let settings_store = SettingsStore::at(config.path().join("settings.json"));
        let state = DesktopState::new_with_settings(
            ConversationStore::at(data.path().to_path_buf()),
            settings_store.clone(),
        );
        let defaults = AtlasSettings::default();

        let result = save_settings_impl(
            SaveSettingsInput {
                api_key: None,
                clear_api_key: false,
                base_url: defaults.deepseek.base_url,
                model: defaults.deepseek.model,
                history_limit: defaults.conversation.history_limit,
                default_workspace_path: None,
            },
            &state,
        );
        match previous {
            Some(value) => std::env::set_var("DEEPSEEK_API_KEY", value),
            None => std::env::remove_var("DEEPSEEK_API_KEY"),
        }

        assert_eq!(result.unwrap().api_key_source, "environment");
        assert_eq!(settings_store.load().unwrap().deepseek.api_key, None);
    }

    #[test]
    fn workspace_validation_returns_canonical_path_and_name() {
        let workspace = TempDir::new().unwrap();

        let validated = validate_workspace_impl(&workspace.path().display().to_string()).unwrap();

        assert_eq!(
            validated.path,
            workspace
                .path()
                .canonicalize()
                .unwrap()
                .display()
                .to_string()
        );
        assert_eq!(
            validated.name,
            workspace.path().file_name().unwrap().to_string_lossy()
        );
    }

    #[test]
    fn invalid_settings_save_preserves_the_previous_file() {
        let data = TempDir::new().unwrap();
        let config = TempDir::new().unwrap();
        let store = ConversationStore::at(data.path().to_path_buf());
        let settings_store = SettingsStore::at(config.path().join("settings.json"));
        let saved = AtlasSettings::default();
        settings_store.save(&saved).unwrap();
        let state = DesktopState::new_with_settings(store, settings_store.clone());

        let error = save_settings_impl(
            SaveSettingsInput {
                api_key: None,
                clear_api_key: false,
                base_url: saved.deepseek.base_url.clone(),
                model: saved.deepseek.model.clone(),
                history_limit: 0,
                default_workspace_path: None,
            },
            &state,
        )
        .unwrap_err();

        assert!(error.contains("history"), "{error}");
        assert_eq!(settings_store.load().unwrap(), saved);
    }

    #[test]
    fn workspace_assignment_returns_workspace_aware_metadata() {
        let data = TempDir::new().unwrap();
        let workspace = TempDir::new().unwrap();
        let store = ConversationStore::at(data.path().to_path_buf());
        let saved = store.create_turn("question", "answer").unwrap();
        let state = DesktopState::new(store);

        let assigned = super::assign_workspace_impl(
            &saved.id.to_string(),
            &workspace.path().display().to_string(),
            &state,
        )
        .unwrap();

        assert_eq!(
            assigned.workspace.unwrap().path,
            workspace
                .path()
                .canonicalize()
                .unwrap()
                .display()
                .to_string()
        );
    }

    #[test]
    fn deleted_saved_workspace_fails_before_runtime_and_history_mutation() {
        let data = TempDir::new().unwrap();
        let config = TempDir::new().unwrap();
        let workspace = TempDir::new().unwrap();
        let workspace_path = workspace.path().to_path_buf();
        let store = ConversationStore::at(data.path().to_path_buf());
        let saved = store
            .create_turn_with_workspace("question", "answer", Some(&workspace_path))
            .unwrap();
        let before = store.load(&saved.id).unwrap();
        drop(workspace);
        let calls = Arc::new(AtomicUsize::new(0));
        let state = DesktopState::with_runtime_factory_and_settings(
            store.clone(),
            SettingsStore::at(config.path().join("settings.json")),
            PathBuf::from("agents/atlas.toml"),
            {
                let calls = Arc::clone(&calls);
                Arc::new(move |_, _| {
                    calls.fetch_add(1, Ordering::SeqCst);
                    Err("runtime must not be built".to_string())
                })
            },
        );

        let error = state
            .ask_with_sink("continue", Some(&saved.id), None, &mut |_| {})
            .unwrap_err();

        assert!(error.contains("workspace"), "{error}");
        assert_eq!(calls.load(Ordering::SeqCst), 0);
        assert_eq!(store.load(&saved.id).unwrap(), before);
    }

    #[test]
    fn deleted_default_workspace_fails_before_runtime_or_conversation_creation() {
        let data = TempDir::new().unwrap();
        let config = TempDir::new().unwrap();
        let workspace = TempDir::new().unwrap();
        let mut settings = AtlasSettings::default();
        settings.workspace.default_path = Some(workspace.path().to_path_buf());
        let settings_store = SettingsStore::at(config.path().join("settings.json"));
        settings_store.save(&settings).unwrap();
        drop(workspace);
        let store = ConversationStore::at(data.path().to_path_buf());
        let calls = Arc::new(AtomicUsize::new(0));
        let state = DesktopState::with_runtime_factory_and_settings(
            store.clone(),
            settings_store,
            PathBuf::from("agents/atlas.toml"),
            {
                let calls = Arc::clone(&calls);
                Arc::new(move |_, _| {
                    calls.fetch_add(1, Ordering::SeqCst);
                    Err("runtime must not be built".to_string())
                })
            },
        );

        let error = state
            .ask_with_sink("start", None, None, &mut |_| {})
            .unwrap_err();

        assert!(error.contains("workspace"), "{error}");
        assert_eq!(calls.load(Ordering::SeqCst), 0);
        assert!(store.list().unwrap().is_empty());
    }

    #[test]
    fn workspace_settings_flow_reconfigures_next_ask_and_never_leaks_the_key() {
        let _guard = environment_lock();
        let previous_key = std::env::var_os("DEEPSEEK_API_KEY");
        std::env::remove_var("DEEPSEEK_API_KEY");
        let root = TempDir::new().unwrap();
        let data_path = root.path().join("data");
        let workspace = root.path().join("workspace");
        let automatic = root.path().join("Atlas");
        std::fs::create_dir(&workspace).unwrap();
        std::fs::create_dir(&automatic).unwrap();
        let store = ConversationStore::at(data_path.clone());
        let settings_store = SettingsStore::at(root.path().join("config/settings.json"));
        let responses = Arc::new(Mutex::new(
            vec![
                FakeResponse {
                    chunks: vec!["first answer"],
                    error: None,
                },
                FakeResponse {
                    chunks: vec!["second answer"],
                    error: None,
                },
            ]
            .into_iter()
            .collect::<VecDeque<_>>(),
        ));
        let histories = Arc::new(Mutex::new(Vec::new()));
        let runtime_calls = Arc::new(Mutex::new(Vec::<(PathBuf, Option<String>)>::new()));
        let state = DesktopState::with_runtime_factory_settings_and_workspace_resolver(
            store.clone(),
            settings_store.clone(),
            PathBuf::from("agents/atlas.toml"),
            {
                let responses = Arc::clone(&responses);
                let histories = Arc::clone(&histories);
                let runtime_calls = Arc::clone(&runtime_calls);
                Arc::new(move |workspace, effective| {
                    runtime_calls
                        .lock()
                        .unwrap()
                        .push((workspace.to_path_buf(), effective.api_key.clone()));
                    Ok(Runtime::new(AgentRegistry::new(vec![Box::new(
                        FakeAgent {
                            responses: Arc::clone(&responses),
                            histories: Arc::clone(&histories),
                        },
                    )])))
                })
            },
            Arc::new(move || Ok(automatic.clone())),
        );
        let secret = "desktop-e2e-secret";

        save_settings_impl(
            SaveSettingsInput {
                api_key: Some(secret.to_string()),
                clear_api_key: false,
                base_url: "https://api.deepseek.com".to_string(),
                model: "deepseek-flash".to_string(),
                history_limit: 40,
                default_workspace_path: None,
            },
            &state,
        )
        .unwrap();
        let first = state
            .ask_with_sink("first question", None, Some(&workspace), &mut |_| {})
            .unwrap();
        let id = ConversationId::parse(&first.metadata.id).unwrap();
        assert_eq!(
            first
                .metadata
                .workspace
                .as_ref()
                .map(|item| item.path.as_str()),
            workspace.canonicalize().unwrap().to_str()
        );

        let loaded = load_impl(&first.metadata.id, &state).unwrap();
        assert_eq!(loaded.messages.len(), 2);
        state
            .ask_with_sink(
                "second question",
                Some(&id),
                Some(Path::new("/frontend/path/is/not/authoritative")),
                &mut |_| {},
            )
            .unwrap();
        let calls = runtime_calls.lock().unwrap().clone();
        assert_eq!(calls.len(), 2);
        assert!(calls.iter().all(|(path, key)| {
            path == &workspace.canonicalize().unwrap() && key.as_deref() == Some(secret)
        }));

        let before_removal = store.load(&id).unwrap();
        std::fs::remove_dir(&workspace).unwrap();
        let error = state
            .ask_with_sink("must not run", Some(&id), None, &mut |_| {})
            .unwrap_err();
        assert!(error.contains("workspace"), "{error}");
        assert!(!error.contains(secret), "{error}");
        assert_eq!(runtime_calls.lock().unwrap().len(), 2);
        assert_eq!(store.load(&id).unwrap(), before_removal);

        let metadata_json = serde_json::to_string(&store.list().unwrap()).unwrap();
        let dto_json = serde_json::to_string(&first.metadata).unwrap();
        let history = std::fs::read_to_string(
            data_path
                .join("conversations")
                .join(format!("{}.jsonl", first.metadata.id)),
        )
        .unwrap();
        assert!(!metadata_json.contains(secret), "{metadata_json}");
        assert!(!dto_json.contains(secret), "{dto_json}");
        assert!(!history.contains(secret), "{history}");

        match previous_key {
            Some(value) => std::env::set_var("DEEPSEEK_API_KEY", value),
            None => std::env::remove_var("DEEPSEEK_API_KEY"),
        }
    }
}
