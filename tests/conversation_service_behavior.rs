use std::{
    cell::{Cell, RefCell},
    collections::VecDeque,
    fs, io,
    rc::Rc,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
};

use atlas_agentic::{
    agents::{AgentRegistry, LocalAgent, LocalAgentDefinition},
    conversations::{
        ActivityCollector, ConversationId, ConversationMessage, ConversationService,
        ConversationStore, MessageRole,
    },
    media::{ImageData, ImageDetail},
    models::{ModelProvider, ModelRequest},
    permissions::{
        AuthorizationDecision, AuthorizationRequest, PermissionMode, ToolAuthorizer, ToolRisk,
    },
    tools::{PwdTool, Tool, ToolDefinition, ToolRegistry, ToolResult},
    Runtime,
};

struct CountingRiskyTool {
    executions: Rc<Cell<usize>>,
}

struct PersistedImageTool;

impl Tool for PersistedImageTool {
    fn name(&self) -> &str {
        "capture_test"
    }

    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: self.name().to_string(),
            description: "Return a synthetic image.".to_string(),
            input_schema: serde_json::json!({"type": "object"}),
        }
    }

    fn execute(&self, _args: &serde_json::Value) -> Result<ToolResult, String> {
        Ok(ToolResult::with_image(
            "Captured synthetic screen.",
            ImageData {
                mime_type: "image/png".to_string(),
                bytes: b"PERSISTENCE_IMAGE_MARKER".to_vec(),
                width: 2,
                height: 1,
                detail: ImageDetail::Original,
            },
        ))
    }
}

struct PersistenceImageModel {
    responses: RefCell<VecDeque<String>>,
    images: Rc<RefCell<Vec<Vec<ImageData>>>>,
}

impl ModelProvider for PersistenceImageModel {
    fn validate_images(&self, _images: &[ImageData]) -> Result<(), String> {
        Ok(())
    }

    fn complete(&self, request: &ModelRequest) -> Result<String, String> {
        if request.tools.is_empty() {
            return Ok("Screen inspection".to_string());
        }
        self.images.borrow_mut().push(request.images.to_vec());
        self.responses
            .borrow_mut()
            .pop_front()
            .ok_or_else(|| "no persistence response queued".to_string())
    }
}

#[test]
fn successful_turn_persists_only_user_and_final_text_not_tool_images() {
    let temp = tempfile::tempdir().unwrap();
    let data = temp.path().join("data");
    let store = ConversationStore::at(data.clone());
    let images = Rc::new(RefCell::new(Vec::new()));
    let model = PersistenceImageModel {
        responses: RefCell::new(
            [
                r#"{"type":"tool_call","tool":"capture_test","args":{}}"#.to_string(),
                r#"{"type":"final","content":"I inspected the screen."}"#.to_string(),
            ]
            .into(),
        ),
        images: Rc::clone(&images),
    };
    let definition = LocalAgentDefinition::from_toml(
        r#"
name = "atlas"
display_name = "Atlas"
model_provider = "fake"
model = "fake"
system_prompt = "You are Atlas."
conversation_title_prompt = "Return a conversation title."
"#,
    )
    .unwrap();
    let runtime = Runtime::new(AgentRegistry::new(vec![Box::new(LocalAgent::new(
        definition,
        Box::new(model),
    ))]))
    .with_tools(ToolRegistry::new(vec![Box::new(PersistedImageTool)]));

    let result = ConversationService::new(&runtime, &store, 40)
        .ask("inspect my screen", None, &mut |_| {})
        .unwrap();

    let saved = store.load(&result.metadata.id).unwrap();
    assert_eq!(saved.messages.len(), 2);
    assert_eq!(saved.messages[0].content, "inspect my screen");
    assert_eq!(saved.messages[1].content, "I inspected the screen.");
    assert_eq!(images.borrow()[1][0].bytes, b"PERSISTENCE_IMAGE_MARKER");
    let jsonl = fs::read_to_string(
        data.join("conversations")
            .join(format!("{}.jsonl", result.metadata.id)),
    )
    .unwrap();
    assert!(!jsonl.contains("PERSISTENCE_IMAGE_MARKER"));
    assert!(!jsonl.contains("data:image"));
}

impl Tool for CountingRiskyTool {
    fn name(&self) -> &str {
        "risky"
    }

    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: self.name().to_string(),
            description: "test tool".to_string(),
            input_schema: serde_json::json!({"type":"object"}),
        }
    }

    fn risk(&self, _args: &serde_json::Value) -> Result<ToolRisk, String> {
        Ok(ToolRisk::ModifyWorkspace)
    }

    fn execute(&self, _args: &serde_json::Value) -> Result<ToolResult, String> {
        self.executions.set(self.executions.get() + 1);
        Ok(ToolResult::text("executed"))
    }
}

struct FixedAuthorizer {
    decision: AuthorizationDecision,
    calls: Cell<usize>,
}

impl ToolAuthorizer for FixedAuthorizer {
    fn authorize(&self, _request: &AuthorizationRequest) -> Result<AuthorizationDecision, String> {
        self.calls.set(self.calls.get() + 1);
        Ok(self.decision)
    }
}

#[derive(Debug)]
struct Request {
    input: String,
    history: Vec<ConversationMessage>,
    activity_context: Option<String>,
    title: bool,
    tool_count: usize,
}

struct Response {
    chunks: Vec<&'static str>,
    error: Option<&'static str>,
}

impl Response {
    fn success(chunks: Vec<&'static str>) -> Self {
        Self {
            chunks,
            error: None,
        }
    }

    fn failure() -> Self {
        Self {
            chunks: vec!["partial answer"],
            error: Some("model unavailable"),
        }
    }
}

struct ScriptedModel {
    responses: RefCell<VecDeque<Response>>,
    requests: Rc<RefCell<Vec<Request>>>,
    title_result: Result<String, String>,
    manual_title_during_generation: Option<&'static str>,
    store: ConversationStore,
}

impl ScriptedModel {
    fn capture(&self, request: &ModelRequest, title: bool) {
        self.requests.borrow_mut().push(Request {
            input: request.user_prompt.to_string(),
            history: request.history.to_vec(),
            activity_context: request.activity_context.map(str::to_string),
            title,
            tool_count: request.tools.len(),
        });
    }
}

impl ModelProvider for ScriptedModel {
    fn complete(&self, request: &ModelRequest) -> Result<String, String> {
        self.capture(request, true);
        assert_eq!(request.system_prompt, "Return a conversation title.");
        // Title generation must happen after the first complete turn is durable.
        let saved = self.store.list().unwrap();
        let metadata = saved
            .iter()
            .find(|metadata| {
                let conversation = self.store.load(&metadata.id).unwrap();
                conversation.messages.len() == 2
                    && conversation.messages[0].content == request.user_prompt
                    && !metadata.title_attempted
            })
            .expect("title generation must follow the persisted first turn");
        if let Some(title) = self.manual_title_during_generation {
            self.store.rename(&metadata.id, title).unwrap();
        }
        self.title_result.clone()
    }

    fn stream(
        &self,
        request: &ModelRequest,
        on_delta: &mut dyn FnMut(&str),
    ) -> Result<String, String> {
        self.capture(request, false);
        let response = self
            .responses
            .borrow_mut()
            .pop_front()
            .expect("unexpected model request");
        for chunk in &response.chunks {
            on_delta(chunk);
        }
        match response.error {
            Some(error) => Err(error.to_string()),
            None => Ok(response.chunks.concat()),
        }
    }
}

fn runtime(
    store: &ConversationStore,
    responses: Vec<Response>,
    title_result: Result<&str, &str>,
) -> (Runtime, Rc<RefCell<Vec<Request>>>) {
    runtime_with_manual_title(store, responses, title_result, None)
}

fn runtime_with_manual_title(
    store: &ConversationStore,
    responses: Vec<Response>,
    title_result: Result<&str, &str>,
    manual_title_during_generation: Option<&'static str>,
) -> (Runtime, Rc<RefCell<Vec<Request>>>) {
    let requests = Rc::new(RefCell::new(Vec::new()));
    let definition = LocalAgentDefinition::from_toml(
        r#"
name = "atlas"
display_name = "Atlas"
model_provider = "fake"
model = "fake"
system_prompt = "You are Atlas."
conversation_title_prompt = "Return a conversation title."
"#,
    )
    .unwrap();
    let model = ScriptedModel {
        responses: RefCell::new(responses.into()),
        requests: Rc::clone(&requests),
        title_result: title_result.map(str::to_string).map_err(str::to_string),
        manual_title_during_generation,
        store: store.clone(),
    };
    let runtime = Runtime::new(AgentRegistry::new(vec![Box::new(LocalAgent::new(
        definition,
        Box::new(model),
    ))]));
    (runtime, requests)
}

fn failing_publication_store(root: std::path::PathBuf, fail_at: usize) -> ConversationStore {
    let publications = Arc::new(AtomicUsize::new(0));
    ConversationStore::at_with_index_publisher(root, move |source, destination| {
        if publications.fetch_add(1, Ordering::SeqCst) + 1 == fail_at {
            Err(io::Error::other("injected publication failure"))
        } else {
            fs::rename(source, destination)
        }
    })
}

#[test]
fn new_ask_streams_before_saving_and_titles_the_persisted_first_turn() {
    let temp = tempfile::tempdir().unwrap();
    let store = ConversationStore::at(temp.path().join("data"));
    let unrelated = store.create_turn("unrelated", "older answer").unwrap();
    let (runtime, requests) = runtime(
        &store,
        vec![Response::success(vec!["hello", " world"])],
        Ok("  \"Atlas\nruntime\"  "),
    );
    let service = ConversationService::new(&runtime, &store, 40);
    let mut streamed = String::new();

    let result = service
        .ask("first input", None, &mut |delta| {
            assert_eq!(store.list().unwrap().len(), 1);
            streamed.push_str(delta);
        })
        .unwrap();

    assert!(result.created);
    assert_eq!(result.answer, "hello world");
    assert_eq!(streamed, result.answer);
    assert_ne!(result.metadata.id, unrelated.id);
    assert_eq!(result.metadata.title, "Atlas runtime");
    assert!(result.metadata.title_attempted);
    assert_eq!(result.metadata.message_count, 2);
    let saved = store.load(&result.metadata.id).unwrap();
    assert_eq!(saved.metadata, result.metadata);
    assert_eq!(saved.messages.len(), 2);
    assert_eq!(saved.messages[0].role, MessageRole::User);
    assert_eq!(saved.messages[0].content, "first input");
    assert_eq!(saved.messages[1].role, MessageRole::Assistant);
    assert_eq!(saved.messages[1].content, "hello world");
    let requests = requests.borrow();
    assert_eq!(requests.len(), 2);
    assert!(!requests[0].title);
    assert!(requests[0].history.is_empty());
    assert_eq!(requests[0].input, "first input");
    assert!(requests[1].title);
    assert_eq!(requests[1].input, "first input");
    assert!(requests[1].history.is_empty());
    assert_eq!(requests[1].tool_count, 0);
}

#[test]
fn model_failure_saves_no_new_conversation_or_partial_answer() {
    let temp = tempfile::tempdir().unwrap();
    let store = ConversationStore::at(temp.path().join("data"));
    let (runtime, requests) = runtime(&store, vec![Response::failure()], Ok("unused"));
    let mut streamed = String::new();

    let error = ConversationService::new(&runtime, &store, 40)
        .ask("first input", None, &mut |delta| streamed.push_str(delta))
        .unwrap_err();

    assert_eq!(error, "model unavailable");
    assert_eq!(streamed, "partial answer");
    assert!(store.list().unwrap().is_empty());
    assert!(!temp.path().join("data/conversations").exists());
    assert_eq!(requests.borrow().len(), 1);
}

#[test]
fn stream_sink_failure_saves_no_conversation() {
    let data = tempfile::tempdir().unwrap();
    let store = ConversationStore::at(data.path().to_path_buf());
    let (runtime, requests) = runtime(
        &store,
        vec![Response::success(vec!["first", " second"])],
        Ok("unused title"),
    );
    let mut deliveries = 0;

    let error = ConversationService::new(&runtime, &store, 40)
        .ask_with_fallible_sink("first input", None, &mut |_| {
            deliveries += 1;
            Err("stream receiver closed".to_string())
        })
        .unwrap_err();

    assert_eq!(error, "stream receiver closed");
    assert_eq!(deliveries, 1);
    assert!(store.list().unwrap().is_empty());
    assert_eq!(requests.borrow().len(), 1);
}

#[test]
fn first_turn_model_and_sink_failures_do_not_persist_workspace_metadata() {
    for sink_fails in [false, true] {
        let temp = tempfile::tempdir().unwrap();
        let workspace = temp.path().join("workspace");
        fs::create_dir(&workspace).unwrap();
        let store = ConversationStore::at(temp.path().join("data"));
        let response = if sink_fails {
            Response::success(vec!["answer"])
        } else {
            Response::failure()
        };
        let (runtime, _) = runtime(&store, vec![response], Ok("unused"));
        let service = ConversationService::new(&runtime, &store, 40);

        let result = service.ask_with_workspace_and_fallible_sink(
            "first input",
            None,
            Some(&workspace),
            &mut |_| {
                if sink_fails {
                    Err("stream receiver closed".to_string())
                } else {
                    Ok(())
                }
            },
        );

        assert!(result.is_err());
        assert!(store.list().unwrap().is_empty());
        assert!(!temp.path().join("data/conversations").exists());
    }
}

#[test]
fn successful_first_turn_persists_the_selected_workspace() {
    let temp = tempfile::tempdir().unwrap();
    let workspace = temp.path().join("workspace");
    fs::create_dir(&workspace).unwrap();
    let store = ConversationStore::at(temp.path().join("data"));
    let (runtime, _) = runtime(&store, vec![Response::success(vec!["answer"])], Ok("title"));

    let result = ConversationService::new(&runtime, &store, 40)
        .ask_with_workspace("first input", None, Some(&workspace), &mut |_| {})
        .unwrap();

    assert_eq!(
        result.metadata.workspace_path,
        Some(workspace.canonicalize().unwrap())
    );
}

#[test]
fn successful_first_turn_persists_permission_but_model_failure_does_not() {
    let temp = tempfile::tempdir().unwrap();
    let store = ConversationStore::at(temp.path().join("data"));
    let (first_runtime, _) = runtime(&store, vec![Response::success(vec!["answer"])], Ok("title"));

    let result = ConversationService::new(&first_runtime, &store, 40)
        .ask_with_workspace_and_permission(
            "first input",
            None,
            None,
            PermissionMode::Workspace,
            &mut |_| {},
        )
        .unwrap();

    assert_eq!(
        result.metadata.permission_mode,
        Some(PermissionMode::Workspace)
    );

    let failed_store = ConversationStore::at(temp.path().join("failed"));
    let (failed_runtime, _) = runtime(&failed_store, vec![Response::failure()], Ok("unused"));
    assert!(ConversationService::new(&failed_runtime, &failed_store, 40)
        .ask_with_workspace_and_permission(
            "first input",
            None,
            None,
            PermissionMode::FullAccess,
            &mut |_| {},
        )
        .is_err());
    assert!(failed_store.list().unwrap().is_empty());
}

#[test]
fn saved_permission_is_authoritative_and_legacy_chat_always_defaults_to_ask() {
    let temp = tempfile::tempdir().unwrap();
    let store = ConversationStore::at(temp.path().join("data"));
    let saved = store
        .create_turn_with_workspace_and_permission(
            "old",
            "answer",
            None,
            Some(PermissionMode::FullAccess),
        )
        .unwrap();
    let executions = Rc::new(Cell::new(0));
    let (saved_runtime, _) = runtime(
        &store,
        vec![
            Response::success(vec![r#"{"type":"tool_call","tool":"risky","args":{}}"#]),
            Response::success(vec![r#"{"type":"final","content":"done"}"#]),
        ],
        Ok("unused"),
    );
    let saved_runtime =
        saved_runtime.with_tools(ToolRegistry::new(vec![Box::new(CountingRiskyTool {
            executions: Rc::clone(&executions),
        })]));
    let authorizer = FixedAuthorizer {
        decision: AuthorizationDecision::Deny,
        calls: Cell::new(0),
    };

    ConversationService::new(&saved_runtime, &store, 40)
        .ask_with_permission_authorizer(
            "continue",
            Some(&saved.id),
            PermissionMode::FullAccess,
            &authorizer,
            &mut |_| {},
            &mut |_| {},
        )
        .unwrap();

    assert_eq!(authorizer.calls.get(), 0);
    assert_eq!(executions.get(), 1);
    assert_eq!(
        store.load(&saved.id).unwrap().metadata.permission_mode,
        Some(PermissionMode::FullAccess)
    );

    let legacy = store.create_turn("legacy", "answer").unwrap();
    let executions = Rc::new(Cell::new(0));
    let (legacy_runtime, _) = runtime(
        &store,
        vec![
            Response::success(vec![r#"{"type":"tool_call","tool":"risky","args":{}}"#]),
            Response::success(vec![r#"{"type":"final","content":"denied"}"#]),
        ],
        Ok("unused"),
    );
    let legacy_runtime =
        legacy_runtime.with_tools(ToolRegistry::new(vec![Box::new(CountingRiskyTool {
            executions: Rc::clone(&executions),
        })]));
    let authorizer = FixedAuthorizer {
        decision: AuthorizationDecision::Deny,
        calls: Cell::new(0),
    };

    ConversationService::new(&legacy_runtime, &store, 40)
        .ask_with_permission_authorizer(
            "continue",
            Some(&legacy.id),
            PermissionMode::FullAccess,
            &authorizer,
            &mut |_| {},
            &mut |_| {},
        )
        .unwrap();

    assert_eq!(authorizer.calls.get(), 1);
    assert_eq!(executions.get(), 0);
    assert_eq!(
        store.load(&legacy.id).unwrap().metadata.permission_mode,
        Some(PermissionMode::Ask)
    );
}

#[test]
fn tool_failure_can_recover_and_save_new_conversation() {
    let temp = tempfile::tempdir().unwrap();
    let store = ConversationStore::at(temp.path().join("data"));
    let (runtime, requests) = runtime(
        &store,
        vec![
            Response::success(vec![r#"{"type":"tool_call","tool":"missing","args":{}}"#]),
            Response::success(vec![r#"{"type":"final","content":"Recovered answer"}"#]),
        ],
        Ok("Recovered conversation"),
    );
    let runtime = runtime.with_tools(ToolRegistry::new(vec![Box::new(PwdTool::new(temp.path()))]));

    let result = ConversationService::new(&runtime, &store, 40)
        .ask("first input", None, &mut |_| {})
        .unwrap();

    assert_eq!(result.answer, "Recovered answer");
    let saved = store.load(&result.metadata.id).unwrap();
    assert_eq!(saved.messages.len(), 2);
    assert_eq!(saved.messages[1].content, "Recovered answer");
    let requests = requests.borrow();
    assert_eq!(requests.len(), 3);
    let retry: serde_json::Value = serde_json::from_str(&requests[1].input).unwrap();
    assert_eq!(retry["tool_results"][0]["success"], false);
    assert_eq!(retry["tool_results"][0]["output"], "unknown tool: missing");
}

#[test]
fn failed_create_returns_storage_error_after_streaming_without_requesting_title() {
    let temp = tempfile::tempdir().unwrap();
    let store = failing_publication_store(temp.path().join("data"), 1);
    let (runtime, requests) = runtime(
        &store,
        vec![Response::success(vec!["saved answer"])],
        Ok("unused"),
    );
    let mut streamed = String::new();

    let error = ConversationService::new(&runtime, &store, 40)
        .ask("first input", None, &mut |delta| streamed.push_str(delta))
        .unwrap_err();

    assert_eq!(streamed, "saved answer");
    assert!(error.contains("injected publication failure"), "{error}");
    assert_eq!(requests.borrow().len(), 1);
}

#[test]
fn title_failure_or_empty_title_keeps_successful_answer_and_records_attempt() {
    for title_result in [Err("title unavailable"), Ok(" \n\"'\"  ")] {
        let temp = tempfile::tempdir().unwrap();
        let store = ConversationStore::at(temp.path().join("data"));
        let (runtime, requests) = runtime(
            &store,
            vec![
                Response::success(vec!["first answer"]),
                Response::success(vec!["second answer"]),
            ],
            title_result,
        );
        let service = ConversationService::new(&runtime, &store, 40);

        let result = service.ask("first input", None, &mut |_| {}).unwrap();

        assert_eq!(result.answer, "first answer");
        assert_eq!(result.metadata.title, "Untitled conversation");
        assert!(result.metadata.title_attempted);
        assert_eq!(
            store.load(&result.metadata.id).unwrap().messages[1].content,
            "first answer"
        );
        let continued = service
            .ask("next input", Some(&result.metadata.id), &mut |_| {})
            .unwrap();
        assert_eq!(continued.metadata.title, "Untitled conversation");
        assert!(continued.metadata.title_attempted);
        assert_eq!(
            requests
                .borrow()
                .iter()
                .filter(|request| request.title)
                .count(),
            1
        );
    }
}

#[test]
fn title_storage_failure_is_explicit_and_preserves_successful_turn() {
    let temp = tempfile::tempdir().unwrap();
    let store = failing_publication_store(temp.path().join("data"), 2);
    let (runtime, requests) = runtime(
        &store,
        vec![Response::success(vec!["first answer"])],
        Err("title unavailable"),
    );

    let error = ConversationService::new(&runtime, &store, 40)
        .ask("first input", None, &mut |_| {})
        .unwrap_err();

    assert!(error.contains("injected publication failure"), "{error}");
    let saved = store.list().unwrap();
    assert_eq!(saved.len(), 1);
    assert_eq!(
        store.load(&saved[0].id).unwrap().messages[1].content,
        "first answer"
    );
    assert_eq!(requests.borrow().len(), 2);
}

#[test]
fn continuation_selects_recent_roles_and_appends_only_final_tool_answer() {
    let temp = tempfile::tempdir().unwrap();
    let store = ConversationStore::at(temp.path().join("data"));
    let metadata = store.create_turn("old input", "old answer").unwrap();
    store
        .append_turn(&metadata.id, "recent input", "recent answer")
        .unwrap();
    let before = store.load(&metadata.id).unwrap();
    let (runtime, requests) = runtime(
        &store,
        vec![
            Response::success(vec![r#"{"type":"tool_call","tool":"pwd","args":{}}"#]),
            Response::success(vec![r#"{"type":"final","content":"final"#, r#" answer"}"#]),
        ],
        Ok("must not title continuation"),
    );
    let runtime = runtime.with_tools(ToolRegistry::new(vec![Box::new(PwdTool::new(temp.path()))]));
    let mut streamed = String::new();

    let result = ConversationService::new(&runtime, &store, 3)
        .ask("next input", Some(&metadata.id), &mut |delta| {
            assert_eq!(store.load(&metadata.id).unwrap(), before);
            streamed.push_str(delta);
        })
        .unwrap();

    assert!(!result.created);
    assert_eq!(result.answer, "final answer");
    assert_eq!(streamed, "final answer");
    assert_eq!(result.metadata.id, metadata.id);
    assert_eq!(result.metadata.created_at, before.metadata.created_at);
    assert!(result.metadata.updated_at >= before.metadata.updated_at);
    assert_eq!(result.metadata.message_count, 6);
    assert!(!result.metadata.title_attempted);
    let saved = store.load(&metadata.id).unwrap();
    assert_eq!(saved.metadata, result.metadata);
    assert_eq!(&saved.messages[..4], before.messages.as_slice());
    assert_eq!(saved.messages[4].role, MessageRole::User);
    assert_eq!(saved.messages[4].content, "next input");
    assert_eq!(saved.messages[5].role, MessageRole::Assistant);
    assert_eq!(saved.messages[5].content, "final answer");
    let requests = requests.borrow();
    assert_eq!(requests.len(), 2);
    for request in requests.iter() {
        assert!(!request.title);
        assert_eq!(request.history, before.messages[2..]);
    }
    assert_eq!(requests[0].input, "next input");
    let continuation: serde_json::Value = serde_json::from_str(&requests[1].input).unwrap();
    assert_eq!(continuation["original_user_request"], "next input");
    assert_eq!(continuation["tool_results"][0]["tool"], "pwd");
}

#[test]
fn continuation_preserves_generated_title_without_another_title_request() {
    let temp = tempfile::tempdir().unwrap();
    let store = ConversationStore::at(temp.path().join("data"));
    let (runtime, requests) = runtime(
        &store,
        vec![
            Response::success(vec!["first answer"]),
            Response::success(vec!["next answer"]),
        ],
        Ok(" \"Stored\ntitle\" "),
    );
    let service = ConversationService::new(&runtime, &store, 40);
    let first = service.ask("first input", None, &mut |_| {}).unwrap();

    let next = service
        .ask("next input", Some(&first.metadata.id), &mut |_| {})
        .unwrap();

    assert_eq!(next.metadata.title, "Stored title");
    assert!(!next.created);
    assert!(next.metadata.title_attempted);
    assert_eq!(
        requests
            .borrow()
            .iter()
            .filter(|request| request.title)
            .count(),
        1
    );
}

#[test]
fn successful_legacy_continuation_assigns_the_resolved_workspace() {
    let temp = tempfile::tempdir().unwrap();
    let workspace = temp.path().join("workspace");
    fs::create_dir(&workspace).unwrap();
    let store = ConversationStore::at(temp.path().join("data"));
    let metadata = store.create_turn("old input", "old answer").unwrap();
    let (runtime, _) = runtime(
        &store,
        vec![Response::success(vec!["next answer"])],
        Ok("unused"),
    );

    let result = ConversationService::new(&runtime, &store, 40)
        .ask_with_workspace(
            "next input",
            Some(&metadata.id),
            Some(&workspace),
            &mut |_| {},
        )
        .unwrap();

    assert_eq!(
        result.metadata.workspace_path,
        Some(workspace.canonicalize().unwrap())
    );
    assert_eq!(store.load(&metadata.id).unwrap().messages.len(), 4);
}

#[test]
fn manual_rename_during_title_generation_wins_over_late_generated_title() {
    for manual_title in ["My chosen title", "Untitled conversation"] {
        let temp = tempfile::tempdir().unwrap();
        let store = ConversationStore::at(temp.path().join("data"));
        let (runtime, requests) = runtime_with_manual_title(
            &store,
            vec![
                Response::success(vec!["first answer"]),
                Response::success(vec!["next answer"]),
            ],
            Ok("Late generated title"),
            Some(manual_title),
        );
        let service = ConversationService::new(&runtime, &store, 40);

        let result = service.ask("first input", None, &mut |_| {}).unwrap();

        assert_eq!(result.metadata.title, manual_title);
        assert!(result.metadata.title_attempted);
        assert_eq!(
            store.load(&result.metadata.id).unwrap().metadata,
            result.metadata
        );
        let continued = service
            .ask("next input", Some(&result.metadata.id), &mut |_| {})
            .unwrap();
        assert_eq!(continued.metadata.title, manual_title);
        assert!(continued.metadata.title_attempted);
        assert_eq!(
            requests
                .borrow()
                .iter()
                .filter(|request| request.title)
                .count(),
            1
        );
    }
}

#[test]
fn failed_continuation_leaves_saved_history_and_metadata_unchanged() {
    for sink_fails in [false, true] {
        let temp = tempfile::tempdir().unwrap();
        let workspace = temp.path().join("workspace");
        fs::create_dir(&workspace).unwrap();
        let store = ConversationStore::at(temp.path().join("data"));
        let metadata = store.create_turn("old input", "old answer").unwrap();
        let before = store.load(&metadata.id).unwrap();
        let response = if sink_fails {
            Response::success(vec!["next answer"])
        } else {
            Response::failure()
        };
        let (runtime, requests) = runtime(&store, vec![response], Ok("unused"));

        let result = ConversationService::new(&runtime, &store, 40)
            .ask_with_workspace_and_fallible_sink(
                "next input",
                Some(&metadata.id),
                Some(&workspace),
                &mut |_| {
                    if sink_fails {
                        Err("stream receiver closed".to_string())
                    } else {
                        Ok(())
                    }
                },
            );

        assert!(result.is_err());
        assert_eq!(store.load(&metadata.id).unwrap(), before);
        assert_eq!(requests.borrow().len(), 1);
    }
}

#[test]
fn failed_append_returns_storage_error_after_answer() {
    let temp = tempfile::tempdir().unwrap();
    let store = failing_publication_store(temp.path().join("data"), 2);
    let metadata = store.create_turn("old input", "old answer").unwrap();
    let (runtime, requests) = runtime(
        &store,
        vec![Response::success(vec!["next answer"])],
        Ok("unused"),
    );
    let mut streamed = String::new();

    let error = ConversationService::new(&runtime, &store, 40)
        .ask("next input", Some(&metadata.id), &mut |delta| {
            streamed.push_str(delta)
        })
        .unwrap_err();

    assert_eq!(streamed, "next answer");
    assert!(error.contains("injected publication failure"), "{error}");
    assert_eq!(requests.borrow().len(), 1);
}

#[test]
fn missing_conversation_is_reported_before_calling_model() {
    let temp = tempfile::tempdir().unwrap();
    let store = ConversationStore::at(temp.path().join("data"));
    let (runtime, requests) = runtime(&store, vec![], Ok("unused"));

    let error = ConversationService::new(&runtime, &store, 40)
        .ask("next input", Some(&ConversationId::new()), &mut |_| {})
        .unwrap_err();

    assert!(error.contains("conversation not found"), "{error}");
    assert!(requests.borrow().is_empty());
}

#[test]
fn successful_service_turn_commits_activity_with_messages() {
    let temp = tempfile::tempdir().unwrap();
    let store = ConversationStore::at(temp.path().join("data"));
    let (runtime, _) = runtime(
        &store,
        vec![Response::success(vec!["completed answer"])],
        Ok("Activity title"),
    );

    let result = ConversationService::new(&runtime, &store, 40)
        .ask("do work", None, &mut |_| {})
        .unwrap();

    let saved = store.load(&result.metadata.id).unwrap();
    assert_eq!(saved.messages.len(), 2);
    assert_eq!(saved.activities.len(), 1);
    assert_eq!(saved.activities[0].records, result.activities);
    assert_eq!(
        saved.activities[0].duration_ms,
        Some(result.activity_duration_ms)
    );
    assert!(result
        .activities
        .iter()
        .any(|record| record.summary == "Thinking"));
}

#[test]
fn failed_service_turn_commits_neither_messages_nor_activity() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("data");
    let store = ConversationStore::at(root.clone());
    let (runtime, _) = runtime(&store, vec![Response::failure()], Ok("unused"));

    assert!(ConversationService::new(&runtime, &store, 40)
        .ask("fail", None, &mut |_| {})
        .is_err());

    assert!(store.list().unwrap().is_empty());
    assert!(!root.join("activities").exists());
}

#[test]
fn continuation_selects_activity_for_the_same_recent_message_window() {
    let temp = tempfile::tempdir().unwrap();
    let store = ConversationStore::at(temp.path().join("data"));
    let mut old = ActivityCollector::new();
    old.status("OLD_ACTIVITY");
    let metadata = store
        .create_turn_with_activity(
            "old input",
            "old answer",
            None,
            None,
            old.into_unassociated_turn(),
        )
        .unwrap();
    let mut recent = ActivityCollector::new();
    recent.status("RECENT_ACTIVITY");
    store
        .append_turn_with_activity(
            &metadata.id,
            "recent input",
            "recent answer",
            None,
            None,
            recent.into_unassociated_turn(),
        )
        .unwrap();
    let (runtime, requests) = runtime(
        &store,
        vec![Response::success(vec!["continued"])],
        Ok("unused"),
    );

    ConversationService::new(&runtime, &store, 2)
        .ask("next", Some(&metadata.id), &mut |_| {})
        .unwrap();

    let requests = requests.borrow();
    let context = requests[0].activity_context.as_deref().unwrap();
    assert!(context.contains("RECENT_ACTIVITY"));
    assert!(!context.contains("OLD_ACTIVITY"));
}

#[test]
fn later_turn_receives_summaries_not_raw_details() {
    let temp = tempfile::tempdir().unwrap();
    let store = ConversationStore::at(temp.path().join("data"));
    let mut activity = ActivityCollector::new();
    activity.tool_finished(
        "search_files",
        true,
        "Searched files successfully",
        "RAW_PRIVATE_DETAIL",
    );
    let metadata = store
        .create_turn_with_activity(
            "old input",
            "old answer",
            None,
            None,
            activity.into_unassociated_turn(),
        )
        .unwrap();
    let (runtime, requests) = runtime(
        &store,
        vec![Response::success(vec!["continued"])],
        Ok("unused"),
    );

    ConversationService::new(&runtime, &store, 40)
        .ask("next", Some(&metadata.id), &mut |_| {})
        .unwrap();

    let context = requests.borrow()[0].activity_context.clone().unwrap();
    assert!(context.contains("Searched files successfully"));
    assert!(!context.contains("RAW_PRIVATE_DETAIL"));
}

#[test]
fn failed_and_denied_tools_are_remembered_after_success() {
    let temp = tempfile::tempdir().unwrap();
    let store = ConversationStore::at(temp.path().join("data"));
    let (runtime, _) = runtime(
        &store,
        vec![
            Response::success(vec![r#"{"type":"tool_call","tool":"missing","args":{}}"#]),
            Response::success(vec![r#"{"type":"tool_call","tool":"risky","args":{}}"#]),
            Response::success(vec![r#"{"type":"final","content":"recovered"}"#]),
        ],
        Ok("Remembered attempts"),
    );
    let runtime = runtime.with_tools(ToolRegistry::new(vec![Box::new(CountingRiskyTool {
        executions: Rc::new(Cell::new(0)),
    })]));
    let authorizer = FixedAuthorizer {
        decision: AuthorizationDecision::Deny,
        calls: Cell::new(0),
    };

    let result = ConversationService::new(&runtime, &store, 40)
        .ask_with_permission_authorizer(
            "try safely",
            None,
            PermissionMode::Ask,
            &authorizer,
            &mut |_| {},
            &mut |_| {},
        )
        .unwrap();

    let saved = store.load(&result.metadata.id).unwrap();
    let summaries = saved.activities[0]
        .records
        .iter()
        .map(|record| record.summary.as_str())
        .collect::<Vec<_>>();
    assert!(summaries.contains(&"missing failed"));
    assert!(summaries.contains(&"risky was denied"));
}

#[test]
fn failed_overall_turn_discards_collected_activity() {
    let temp = tempfile::tempdir().unwrap();
    let store = ConversationStore::at(temp.path().join("data"));
    let metadata = store.create_turn("old", "answer").unwrap();
    let (runtime, _) = runtime(
        &store,
        vec![
            Response::success(vec![r#"{"type":"tool_call","tool":"pwd","args":{}}"#]),
            Response::failure(),
        ],
        Ok("unused"),
    );
    let runtime = runtime.with_tools(ToolRegistry::new(vec![Box::new(PwdTool::new(temp.path()))]));

    assert!(ConversationService::new(&runtime, &store, 40)
        .ask("continue", Some(&metadata.id), &mut |_| {})
        .is_err());

    let saved = store.load(&metadata.id).unwrap();
    assert_eq!(saved.messages.len(), 2);
    assert!(saved.activities.is_empty());
}

#[test]
fn progress_is_saved_as_activity_and_returned_to_later_model_context() {
    use atlas_agentic::{conversations::ActivityKind, tools::UpdateProgressTool};
    let temp = tempfile::tempdir().unwrap();
    let store = ConversationStore::at(temp.path().join("data"));
    let (runtime, requests) = runtime(
        &store,
        vec![
            Response::success(vec![
                r#"{"type":"tool_call","tool":"update_progress","args":{"message":"Unvalidated preview"#,
            ]),
            Response::success(vec![
                r#"{"type":"tool_call","tool":"update_progress","args":{"message":"I will inspect the files."}}"#,
            ]),
            Response::success(vec![
                r#"{"type":"tool_call","tool":"update_progress","args":{"message":"I will run the tests next."}}"#,
            ]),
            Response::success(vec![
                r#"{"type":"final","content":"Finished and verified."}"#,
            ]),
            Response::success(vec![
                r#"{"type":"final","content":"Continuing with prior context."}"#,
            ]),
        ],
        Ok("Progress task"),
    );
    let runtime = runtime.with_tools(ToolRegistry::new(vec![Box::new(UpdateProgressTool::new())]));
    let service = ConversationService::new(&runtime, &store, 40);
    let result = service.ask("do work", None, &mut |_| {}).unwrap();
    let saved = store.load(&result.metadata.id).unwrap();
    assert_eq!(saved.messages.len(), 2);
    assert_eq!(saved.messages[1].content, "Finished and verified.");
    let progress: Vec<_> = saved.activities[0]
        .records
        .iter()
        .filter(|r| r.kind == ActivityKind::Progress)
        .collect();
    assert_eq!(progress.len(), 2);
    assert!(progress[0].summary.contains("I will inspect the files."));
    assert!(progress[1].summary.contains("I will run the tests next."));
    let json = fs::read_to_string(
        temp.path()
            .join("data/activities")
            .join(format!("{}.jsonl", result.metadata.id)),
    )
    .unwrap();
    assert!(!json.contains("Unvalidated preview"));
    assert!(json.contains("I will inspect the files."));
    service
        .ask("continue", Some(&result.metadata.id), &mut |_| {})
        .unwrap();
    let requests = requests.borrow();
    let later = requests.iter().rfind(|request| !request.title).unwrap();
    assert_eq!(later.history.len(), 2);
    let activity = later.activity_context.as_deref().unwrap();
    assert!(activity.contains("I will inspect the files."));
    assert!(activity.contains("I will run the tests next."));
    assert!(activity.contains("not a completed action"));
}

#[test]
fn failed_turn_does_not_persist_progress_as_chat_or_activity() {
    use atlas_agentic::tools::UpdateProgressTool;
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("data");
    let store = ConversationStore::at(root.clone());
    let (runtime, _) = runtime(
        &store,
        vec![
            Response::success(vec![
                r#"{"type":"tool_call","tool":"update_progress","args":{"message":"I will inspect the files."}}"#,
            ]),
            Response::failure(),
        ],
        Ok("unused"),
    );
    let runtime = runtime.with_tools(ToolRegistry::new(vec![Box::new(UpdateProgressTool::new())]));
    let error = ConversationService::new(&runtime, &store, 40)
        .ask("do work", None, &mut |_| {})
        .unwrap_err();
    assert_eq!(error, "model unavailable");
    assert!(store.list().unwrap().is_empty());
    assert!(!root.join("activities").exists());
}
