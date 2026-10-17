use atlas_agentic::{
    agents::{AgentRegistry, LocalAgent, LocalAgentDefinition},
    conversations::*,
    models::*,
    Runtime,
};
use std::{cell::RefCell, rc::Rc};
struct Fake {
    summaries: Rc<RefCell<Vec<String>>>,
    fail: bool,
    finish: &'static str,
    summary: &'static str,
}
impl ModelProvider for Fake {
    fn complete(&self, _: &ModelRequest) -> Result<String, String> {
        Ok("title".into())
    }
    fn stream(&self, _: &ModelRequest, _: &mut dyn FnMut(&str)) -> Result<String, String> {
        Ok(r#"{"type":"final","content":"done"}"#.into())
    }
    fn model_limits(&self) -> Result<Option<ModelLimits>, String> {
        if self.finish == "limits-unavailable" {
            return Err("limits unavailable".into());
        }
        Ok(Some(ModelLimits {
            context_window: 100_000,
            max_output_tokens: 4096,
        }))
    }
    fn estimate_request(&self, r: &ModelRequest) -> Result<Option<TokenEstimate>, String> {
        Ok(Some(TokenEstimate {
            tokens: r.user_prompt.len() as u64
                + r.history
                    .iter()
                    .map(|m| m.content.len() as u64)
                    .sum::<u64>(),
            estimated: true,
        }))
    }
    fn summarize(&self, r: &SummaryRequest) -> Result<SummaryResponse, String> {
        self.summaries.borrow_mut().push(r.content.into());
        if self.fail && self.summaries.borrow().len() == 3 {
            return Err("block failed".into());
        }
        Ok(SummaryResponse {
            content: self.summary.into(),
            usage: None,
            finish_reason: self.finish.into(),
        })
    }
}
fn runtime(summaries: Rc<RefCell<Vec<String>>>, fail: bool) -> Runtime {
    runtime_with_output(summaries, fail, "stop", "Useful summary")
}
fn runtime_with_output(
    summaries: Rc<RefCell<Vec<String>>>,
    fail: bool,
    finish: &'static str,
    summary: &'static str,
) -> Runtime {
    let definition = LocalAgentDefinition::from_toml(
        r#"
name="fake"
display_name="Fake"
model_provider="fake"
model="fake"
system_prompt="assistant"
conversation_title_prompt="title"
conversation_summary_prompt="summarize history"
[memory]
enabled=true
"#,
    )
    .unwrap();
    Runtime::new(AgentRegistry::new(vec![Box::new(LocalAgent::new(
        definition,
        Box::new(Fake {
            summaries,
            fail,
            finish,
            summary,
        }),
    ))]))
}
#[test]
fn manual_compaction_saves_one_message_below_thirty_percent_and_preserves_latest_ten() {
    let data = tempfile::tempdir().unwrap();
    let store = ConversationStore::at(data.path().into());
    let meta = store.create_turn("one", "two").unwrap();
    for _ in 0..9 {
        store.append_turn(&meta.id, "more", "answer").unwrap();
    }
    let calls = Rc::new(RefCell::new(vec![]));
    let runtime = runtime(calls.clone(), false);
    let service = ConversationService::new(&runtime, &store, 40);
    let result = service.compact(&meta.id, &mut |_| {}).unwrap();
    let memory = result.memory.unwrap();
    assert_eq!(memory.blocks.len(), 1);
    assert_eq!(memory.compacted_message_count, 10);
    assert_eq!(store.load(&meta.id).unwrap().messages.len(), 20);
    assert_eq!(store.load_memory(&meta.id).unwrap(), Some(memory));
    assert_eq!(calls.borrow().len(), 1);
    service.ask("next", Some(&meta.id), &mut |_| {}).unwrap();
    assert_eq!(store.load(&meta.id).unwrap().messages.len(), 22);
}
#[test]
fn failed_third_block_does_not_publish_partial_memory() {
    let data = tempfile::tempdir().unwrap();
    let store = ConversationStore::at(data.path().into());
    let meta = store
        .create_turn(&"x".repeat(4000), &"y".repeat(4000))
        .unwrap();
    for _ in 0..9 {
        store
            .append_turn(&meta.id, &"x".repeat(4000), &"y".repeat(4000))
            .unwrap();
    }
    let calls = Rc::new(RefCell::new(vec![]));
    let runtime = runtime(calls, true);
    let service = ConversationService::new(&runtime, &store, 40);
    assert!(service.compact(&meta.id, &mut |_| {}).is_err());
    assert!(store.load_memory(&meta.id).unwrap().is_none());
}

#[test]
fn incomplete_empty_and_cancelled_summaries_keep_original_history() {
    for (finish, summary, cancelled) in [
        ("length", "partial", false),
        ("", "unknown", false),
        ("stop", "", false),
        ("stop", "useful", true),
    ] {
        let data = tempfile::tempdir().unwrap();
        let store = ConversationStore::at(data.path().into());
        let meta = store.create_turn("first", "answer").unwrap();
        for _ in 0..6 {
            store.append_turn(&meta.id, "more", "answer").unwrap();
        }
        let calls = Rc::new(RefCell::new(vec![]));
        let runtime = runtime_with_output(calls.clone(), false, finish, summary);
        let service = ConversationService::new(&runtime, &store, 40);
        let flag = std::sync::atomic::AtomicBool::new(cancelled);
        let error = service
            .compact_cancellable(&meta.id, &mut |_| {}, Some(&flag))
            .unwrap_err();
        let expected = if cancelled {
            "cancelled"
        } else if finish == "length" {
            "output limit"
        } else if finish != "stop" {
            "finish reason"
        } else {
            "empty"
        };
        assert!(error.contains(expected), "expected {expected}: {error}");
        assert!(store.load_memory(&meta.id).unwrap().is_none());
        assert_eq!(store.load(&meta.id).unwrap().messages.len(), 14);
        if cancelled {
            assert!(calls.borrow().is_empty());
        }
    }
}

#[test]
fn oversized_unicode_source_reduces_in_bounded_pieces_to_one_block() {
    let data = tempfile::tempdir().unwrap();
    let store = ConversationStore::at(data.path().into());
    let meta = store
        .create_turn(&"🌍".repeat(25000), &"🌍".repeat(25000))
        .unwrap();
    for _ in 0..5 {
        store.append_turn(&meta.id, "recent", "answer").unwrap();
    }
    let calls = Rc::new(RefCell::new(vec![]));
    let runtime = runtime(calls.clone(), false);
    let result = ConversationService::new(&runtime, &store, 40)
        .compact(&meta.id, &mut |_| {})
        .unwrap();
    assert_eq!(result.memory.unwrap().blocks.len(), 1);
    assert!(calls.borrow().len() > 1);
    assert!(calls.borrow().len() <= 24);
    assert_eq!(store.load(&meta.id).unwrap().messages.len(), 12);
}

#[test]
fn short_history_is_a_noop_without_discovering_model_limits() {
    let data = tempfile::tempdir().unwrap();
    let store = ConversationStore::at(data.path().into());
    let meta = store.create_turn("first", "answer").unwrap();
    let calls = Rc::new(RefCell::new(vec![]));
    let runtime = runtime_with_output(calls.clone(), false, "limits-unavailable", "");
    let result = ConversationService::new(&runtime, &store, 40)
        .compact(&meta.id, &mut |_| {})
        .unwrap();
    assert!(result.memory.is_none());
    assert!(calls.borrow().is_empty());
}

#[test]
fn manual_recompaction_can_replace_existing_memory_without_new_older_messages() {
    let data = tempfile::tempdir().unwrap();
    let store = ConversationStore::at(data.path().into());
    let meta = store.create_turn("first", "answer").unwrap();
    for _ in 0..6 {
        store.append_turn(&meta.id, "more", "answer").unwrap();
    }
    let runtime = runtime(Default::default(), false);
    let service = ConversationService::new(&runtime, &store, 40);
    let first = service
        .compact(&meta.id, &mut |_| {})
        .unwrap()
        .memory
        .unwrap();
    let second = service
        .compact(&meta.id, &mut |_| {})
        .unwrap()
        .memory
        .unwrap();
    assert_ne!(first.checkpoint_id, second.checkpoint_id);
    assert_eq!(
        first.compacted_message_count,
        second.compacted_message_count
    );
}

#[test]
fn truncated_summary_keeps_an_existing_checkpoint_and_full_history() {
    let data = tempfile::tempdir().unwrap();
    let store = ConversationStore::at(data.path().into());
    let meta = store.create_turn("first", "answer").unwrap();
    for _ in 0..9 {
        store.append_turn(&meta.id, "more", "answer").unwrap();
    }
    let good = runtime(Default::default(), false);
    ConversationService::new(&good, &store, 40)
        .compact(&meta.id, &mut |_| {})
        .unwrap();
    let before = store.load(&meta.id).unwrap();
    let broken = runtime_with_output(Default::default(), false, "length", "partial");
    let error = ConversationService::new(&broken, &store, 40)
        .compact(&meta.id, &mut |_| {})
        .unwrap_err();
    assert!(error.contains("output limit"));
    assert_eq!(store.load(&meta.id).unwrap(), before);
}
