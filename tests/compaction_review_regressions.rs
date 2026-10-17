use atlas_agentic::{
    agents::{AgentRegistry, LocalAgent, LocalAgentDefinition},
    conversations::*,
    models::*,
    Runtime,
};
use std::{
    cell::RefCell,
    rc::Rc,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};
struct Model {
    budgets: Rc<RefCell<Vec<u64>>>,
    inputs: Rc<RefCell<Vec<String>>>,
    cancel: Option<Arc<AtomicBool>>,
}
impl ModelProvider for Model {
    fn complete(&self, _: &ModelRequest) -> Result<String, String> {
        Ok("title".into())
    }
    fn stream(&self, _: &ModelRequest, _: &mut dyn FnMut(&str)) -> Result<String, String> {
        Ok("done".into())
    }
    fn model_limits(&self) -> Result<Option<ModelLimits>, String> {
        Ok(Some(ModelLimits {
            context_window: 100_000,
            max_output_tokens: 4096,
        }))
    }
    fn estimate_request(&self, r: &ModelRequest) -> Result<Option<TokenEstimate>, String> {
        Ok(Some(TokenEstimate {
            tokens: (r.user_prompt.len()
                + r.history.iter().map(|m| m.content.len()).sum::<usize>()
                + r.memory_blocks
                    .iter()
                    .map(|b| b.content.len())
                    .sum::<usize>()) as u64,
            estimated: false,
        }))
    }
    fn summarize(&self, r: &SummaryRequest) -> Result<SummaryResponse, String> {
        self.budgets.borrow_mut().push(r.max_output_tokens);
        self.inputs.borrow_mut().push(r.content.into());
        if let Some(flag) = &self.cancel {
            flag.store(true, Ordering::Relaxed)
        }
        Ok(SummaryResponse {
            content: "s".repeat(r.max_output_tokens as usize),
            usage: None,
            finish_reason: "stop".into(),
        })
    }
}
fn runtime(model: Model) -> Runtime {
    let definition = LocalAgentDefinition::from_toml(
        r#"
name="fake"
display_name="Fake"
model_provider="fake"
model="fake"
system_prompt="assistant"
conversation_title_prompt="title"
conversation_summary_prompt="summarize"
[memory]
enabled=true
"#,
    )
    .unwrap();
    Runtime::new(AgentRegistry::new(vec![Box::new(LocalAgent::new(
        definition,
        Box::new(model),
    ))]))
}
#[test]
fn summary_output_budgets_share_only_the_space_left_after_recent_context() {
    let data = tempfile::tempdir().unwrap();
    let store = ConversationStore::at(data.path().into());
    let meta = store
        .create_turn(&"x".repeat(3000), &"y".repeat(3000))
        .unwrap();
    for _ in 0..9 {
        store
            .append_turn(&meta.id, &"x".repeat(3000), &"y".repeat(3000))
            .unwrap();
    }
    let budgets = Rc::new(RefCell::new(vec![]));
    let runtime = runtime(Model {
        budgets: budgets.clone(),
        inputs: Default::default(),
        cancel: None,
    });
    ConversationService::new(&runtime, &store, 40)
        .ask(&"z".repeat(55000), Some(&meta.id), &mut |_| {})
        .unwrap();
    assert_eq!(budgets.borrow().len(), 3);
    assert!(budgets.borrow().iter().all(|n| *n <= 301));
    assert_eq!(
        store.load_memory(&meta.id).unwrap().unwrap().blocks.len(),
        3
    );
}
#[test]
fn automatic_compaction_cancelled_after_a_summary_does_not_publish_or_save_answer() {
    let data = tempfile::tempdir().unwrap();
    let store = ConversationStore::at(data.path().into());
    let meta = store
        .create_turn(&"x".repeat(3000), &"y".repeat(3000))
        .unwrap();
    for _ in 0..9 {
        store
            .append_turn(&meta.id, &"x".repeat(3000), &"y".repeat(3000))
            .unwrap();
    }
    let flag = Arc::new(AtomicBool::new(false));
    let runtime = runtime(Model {
        budgets: Default::default(),
        inputs: Default::default(),
        cancel: Some(flag.clone()),
    });
    assert!(ConversationService::new(&runtime, &store, 40)
        .with_cancellation(&flag)
        .ask(&"z".repeat(40000), Some(&meta.id), &mut |_| {})
        .is_err());
    assert!(store.load_memory(&meta.id).unwrap().is_none());
    assert_eq!(store.load(&meta.id).unwrap().messages.len(), 20);
}
#[test]
fn every_oversized_fragment_keeps_its_speaker_and_source_range() {
    let data = tempfile::tempdir().unwrap();
    let store = ConversationStore::at(data.path().into());
    let meta = store.create_turn("user goal", &"🌍".repeat(50000)).unwrap();
    for _ in 0..5 {
        store.append_turn(&meta.id, "recent", "answer").unwrap();
    }
    let inputs = Rc::new(RefCell::new(vec![]));
    let runtime = runtime(Model {
        budgets: Default::default(),
        inputs: inputs.clone(),
        cancel: None,
    });
    let _ = ConversationService::new(&runtime, &store, 40).compact(&meta.id, &mut |_| {});
    let requests = inputs.borrow();
    assert!(requests.len() > 1);
    for content in requests.iter() {
        assert!(content.contains("\"sourceRoles\""));
        assert!(content.contains("\"startIndex\""));
        assert!(content.contains("\"endIndexExclusive\""));
    }
    assert!(requests
        .iter()
        .any(|r| r.contains("\"sourceRoles\":[\"assistant\"]")));
}
