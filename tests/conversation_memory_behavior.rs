use atlas_agentic::{
    agents::{AgentRegistry, LocalAgent, LocalAgentDefinition},
    conversations::*,
    models::*,
    tools::{PwdTool, ToolRegistry},
    Runtime,
};
use std::{cell::RefCell, rc::Rc};
#[derive(Default)]
struct Observations {
    requests: Vec<(usize, usize, String)>,
    summaries: usize,
}
struct Model(Rc<RefCell<Observations>>);
impl ModelProvider for Model {
    fn complete(&self, _: &ModelRequest) -> Result<String, String> {
        Ok("title".into())
    }
    fn stream(&self, r: &ModelRequest, d: &mut dyn FnMut(&str)) -> Result<String, String> {
        self.stream_with_usage(r, d).map(|r| r.content)
    }
    fn model_name(&self) -> Option<&str> {
        Some("selected-model")
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
            estimated: true,
        }))
    }
    fn summarize(&self, _: &SummaryRequest) -> Result<SummaryResponse, String> {
        self.0.borrow_mut().summaries += 1;
        Ok(SummaryResponse {
            content: "goals and findings".into(),
            usage: None,
            finish_reason: "stop".into(),
        })
    }
    fn stream_with_usage(
        &self,
        r: &ModelRequest,
        d: &mut dyn FnMut(&str),
    ) -> Result<ModelResponse, String> {
        let mut o = self.0.borrow_mut();
        o.requests
            .push((r.memory_blocks.len(), r.history.len(), r.user_prompt.into()));
        let first = o.requests.len() == 1;
        let content = if first {
            r#"{"type":"tool_call","tool":"pwd","args":{}}"#
        } else {
            r#"{"type":"final","content":"done"}"#
        }
        .to_string();
        d(&content);
        Ok(ModelResponse {
            content,
            usage: Some(ModelUsage {
                model: "selected-model".into(),
                input_tokens: if first { 94_000 } else { 30_000 },
                output_tokens: 1000,
                total_tokens: if first { 95_000 } else { 31_000 },
            }),
        })
    }
}
#[test]
fn automatic_compaction_between_tool_calls_preserves_results_and_reopens_memory() {
    let data = tempfile::tempdir().unwrap();
    let store = ConversationStore::at(data.path().join("history"));
    let meta = store
        .create_turn(&"x".repeat(3000), &"y".repeat(3000))
        .unwrap();
    for _ in 0..9 {
        store
            .append_turn(&meta.id, &"x".repeat(3000), &"y".repeat(3000))
            .unwrap();
    }
    let definition = LocalAgentDefinition::from_toml(
        r#"
name="fake"
display_name="Fake"
model_provider="fake"
model="configured-name"
system_prompt="assistant"
conversation_title_prompt="title"
conversation_summary_prompt="summarize"
[memory]
enabled=true
"#,
    )
    .unwrap();
    let observations = Rc::new(RefCell::new(Observations::default()));
    let runtime = Runtime::new(AgentRegistry::new(vec![Box::new(LocalAgent::new(
        definition,
        Box::new(Model(observations.clone())),
    ))]))
    .with_tools(ToolRegistry::new(vec![Box::new(PwdTool::new(data.path()))]));
    let service = ConversationService::new(&runtime, &store, 40);
    let answer = service.ask("task", Some(&meta.id), &mut |_| {}).unwrap();
    assert_eq!(answer.answer, "done");
    let loaded = store.load(&meta.id).unwrap();
    let cp = loaded.memory.unwrap();
    assert_eq!(cp.blocks.len(), 3);
    assert_eq!(cp.compacted_message_count, 10);
    assert_eq!(cp.model, "selected-model");
    let o = observations.borrow();
    assert_eq!(o.requests[0].0, 0);
    assert_eq!(o.requests[0].1, 20);
    assert_eq!(o.requests[1].0, 3);
    assert_eq!(o.requests[1].1, 10);
    assert!(o.requests[1].2.contains("tool_results"));
    assert_eq!(o.summaries, 3);
    drop(o);
    service
        .ask("continue", Some(&meta.id), &mut |_| {})
        .unwrap();
    assert_eq!(observations.borrow().requests[2].0, 3);
    assert_eq!(observations.borrow().requests[2].1, 12);
    let second = service
        .compact(&meta.id, &mut |_| {})
        .unwrap()
        .memory
        .unwrap();
    assert_ne!(second.checkpoint_id, cp.checkpoint_id);
    assert_eq!(second.blocks.len(), 2);
    assert_eq!(second.compacted_message_count, 14);
    assert_eq!(second.blocks[0].start_index, 0);
    assert_eq!(second.blocks.last().unwrap().end_index_exclusive, 14);
}

#[test]
fn full_history_exceeds_legacy_forty_and_unknown_capacity_is_reported() {
    use std::cell::Cell;
    struct HistoryModel(Rc<Cell<usize>>);
    impl ModelProvider for HistoryModel {
        fn complete(&self, _: &ModelRequest) -> Result<String, String> {
            Ok("title".into())
        }
        fn stream(&self, r: &ModelRequest, _: &mut dyn FnMut(&str)) -> Result<String, String> {
            self.0.set(r.history.len());
            Ok("answer".into())
        }
    }
    let data = tempfile::tempdir().unwrap();
    let store = ConversationStore::at(data.path().into());
    let meta = store.create_turn("first", "answer").unwrap();
    for _ in 0..29 {
        store.append_turn(&meta.id, "more", "answer").unwrap();
    }
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
    let seen = Rc::new(Cell::new(0));
    let runtime = Runtime::new(AgentRegistry::new(vec![Box::new(LocalAgent::new(
        definition,
        Box::new(HistoryModel(seen.clone())),
    ))]));
    let mut unavailable = false;
    ConversationService::new(&runtime, &store, 40)
        .ask_with_permission_authorizer(
            "next",
            Some(&meta.id),
            atlas_agentic::permissions::PermissionMode::Ask,
            &atlas_agentic::permissions::AllowAllAuthorizer,
            &mut |_| {},
            &mut |event| {
                if let atlas_agentic::permissions::ToolEvent::ContextStatus { status } = event {
                    unavailable = !status.available;
                }
            },
        )
        .unwrap();
    assert_eq!(seen.get(), 60);
    assert!(unavailable);
}

#[test]
fn usage_from_another_model_cannot_trigger_compaction_before_the_first_call() {
    let data = tempfile::tempdir().unwrap();
    let store = ConversationStore::at(data.path().into());
    let meta = store.create_turn("first", "answer").unwrap();
    for _ in 0..9 {
        let mut activity = ActivityCollector::new().into_unassociated_turn();
        activity.usage = Some(ModelUsage {
            model: "other-model".into(),
            input_tokens: 94000,
            output_tokens: 1000,
            total_tokens: 95000,
        });
        store
            .append_turn_with_activity(&meta.id, "more", "answer", None, None, activity)
            .unwrap();
    }
    let definition = LocalAgentDefinition::from_toml(
        r#"
name="fake"
display_name="Fake"
model_provider="fake"
model="configured-name"
system_prompt="assistant"
conversation_title_prompt="title"
conversation_summary_prompt="summarize"
[memory]
enabled=true
"#,
    )
    .unwrap();
    let observations = Rc::new(RefCell::new(Observations::default()));
    let runtime = Runtime::new(AgentRegistry::new(vec![Box::new(LocalAgent::new(
        definition,
        Box::new(Model(observations.clone())),
    ))]))
    .with_tools(ToolRegistry::new(vec![Box::new(PwdTool::new(data.path()))]));
    ConversationService::new(&runtime, &store, 40)
        .ask("next", Some(&meta.id), &mut |_| {})
        .unwrap();
    assert_eq!(observations.borrow().requests[0].0, 0);
}
