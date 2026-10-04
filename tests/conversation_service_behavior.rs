use std::{
    cell::RefCell,
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
        ConversationId, ConversationMessage, ConversationService, ConversationStore, MessageRole,
    },
    models::{ModelProvider, ModelRequest},
    tools::{PwdTool, ToolRegistry},
    Runtime,
};

#[derive(Debug)]
struct Request {
    input: String,
    history: Vec<ConversationMessage>,
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
fn tool_failure_saves_no_new_conversation() {
    let temp = tempfile::tempdir().unwrap();
    let store = ConversationStore::at(temp.path().join("data"));
    let (runtime, requests) = runtime(
        &store,
        vec![Response::success(vec![
            r#"{"type":"tool_call","tool":"missing","args":{}}"#,
        ])],
        Ok("unused"),
    );
    let runtime = runtime.with_tools(ToolRegistry::new(vec![Box::new(PwdTool::new(temp.path()))]));

    let error = ConversationService::new(&runtime, &store, 40)
        .ask("first input", None, &mut |_| {})
        .unwrap_err();

    assert!(error.contains("unknown tool"), "{error}");
    assert!(store.list().unwrap().is_empty());
    assert!(!temp.path().join("data/conversations").exists());
    assert_eq!(requests.borrow().len(), 1);
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
    let temp = tempfile::tempdir().unwrap();
    let store = ConversationStore::at(temp.path().join("data"));
    let metadata = store.create_turn("old input", "old answer").unwrap();
    let before = store.load(&metadata.id).unwrap();
    let (runtime, requests) = runtime(&store, vec![Response::failure()], Ok("unused"));

    let error = ConversationService::new(&runtime, &store, 40)
        .ask("next input", Some(&metadata.id), &mut |_| {})
        .unwrap_err();

    assert_eq!(error, "model unavailable");
    assert_eq!(store.load(&metadata.id).unwrap(), before);
    assert_eq!(requests.borrow().len(), 1);
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
