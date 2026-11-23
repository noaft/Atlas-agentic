use atlas_agentic::{
    agents::{AgentRegistry, LocalAgent, LocalAgentDefinition},
    conversations::{ConversationService, ConversationStore},
    models::{ModelProvider, ModelRequest, TokenEstimate},
    tools::{Tool, ToolDefinition, ToolRegistry, ToolResult},
    Runtime,
};
use serde_json::json;
use std::{cell::RefCell, rc::Rc};
struct Provider {
    events: Rc<RefCell<Vec<String>>>,
    fail_count: bool,
}
impl ModelProvider for Provider {
    fn complete(&self, _: &ModelRequest) -> Result<String, String> {
        Ok("title".into())
    }
    fn estimate_request(&self, r: &ModelRequest) -> Result<Option<TokenEstimate>, String> {
        self.events
            .borrow_mut()
            .push(format!("count:{}", r.user_prompt));
        if self.fail_count {
            return Err("count unavailable".into());
        }
        Ok(Some(TokenEstimate {
            tokens: 10,
            estimated: false,
        }))
    }
    fn stream(&self, r: &ModelRequest, _: &mut dyn FnMut(&str)) -> Result<String, String> {
        self.events
            .borrow_mut()
            .push(format!("send:{}", r.user_prompt));
        let calls = self
            .events
            .borrow()
            .iter()
            .filter(|e| e.starts_with("send:"))
            .count();
        Ok(match calls {
            1 => r#"{"type":"tool_call","tool":"sample","args":{}}"#,
            2 => "invalid protocol",
            _ => r#"{"type":"final","content":"done"}"#,
        }
        .into())
    }
}
struct Sample;
impl Tool for Sample {
    fn name(&self) -> &str {
        "sample"
    }
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "sample".into(),
            description: "read".into(),
            input_schema: json!({"type":"object"}),
        }
    }
    fn execute(&self, _: &serde_json::Value) -> Result<ToolResult, String> {
        Ok(ToolResult::text("tool evidence"))
    }
}
fn runtime(events: Rc<RefCell<Vec<String>>>, fail_count: bool) -> Runtime {
    let definition=LocalAgentDefinition::from_toml("name='fake'\ndisplay_name='Fake'\nmodel_provider='fake'\nmodel='fake'\nsystem_prompt='assistant'\nconversation_title_prompt='title'\nconversation_summary_prompt='summary'\n[memory]\nenabled=true").unwrap();
    Runtime::new(AgentRegistry::new(vec![Box::new(LocalAgent::new(
        definition,
        Box::new(Provider { events, fail_count }),
    ))]))
    .with_tools(ToolRegistry::new(vec![Box::new(Sample)]))
}
#[test]
fn every_inference_counts_current_payload_even_when_capacity_is_unknown() {
    let events = Rc::new(RefCell::new(vec![]));
    let runtime = runtime(events.clone(), false);
    let data = tempfile::tempdir().unwrap();
    let store = ConversationStore::at(data.path().into());
    ConversationService::new(&runtime, &store, 40)
        .ask("task", None, &mut |_| {})
        .unwrap();
    let events = events.borrow();
    let mut last_count = None;
    let mut sent = Vec::new();
    for event in events.iter() {
        if let Some(payload) = event.strip_prefix("count:") {
            last_count = Some(payload);
        }
        if let Some(payload) = event.strip_prefix("send:") {
            assert_eq!(
                last_count,
                Some(payload),
                "each inference must preflight its current payload"
            );
            sent.push(payload);
        }
    }
    assert_eq!(sent.len(), 3);
    assert!(sent[1].contains("tool evidence"));
    assert!(sent[2].contains("invalid protocol"));
    assert_eq!(sent[2].matches("tool evidence").count(), 1);
}
#[test]
fn failed_count_blocks_inference_and_does_not_save_a_turn() {
    let events = Rc::new(RefCell::new(vec![]));
    let runtime = runtime(events.clone(), true);
    let data = tempfile::tempdir().unwrap();
    let store = ConversationStore::at(data.path().into());
    let error = ConversationService::new(&runtime, &store, 40)
        .ask("task", None, &mut |_| {})
        .unwrap_err();
    assert_eq!(error, "count unavailable");
    assert!(events.borrow().iter().all(|e| !e.starts_with("send:")));
    assert!(store.list().unwrap().is_empty());
}

#[test]
fn direct_runtime_also_preflights_each_tool_and_repair_continuation() {
    let events = Rc::new(RefCell::new(vec![]));
    let runtime = runtime(events.clone(), false);
    assert_eq!(runtime.ask_stream("task", &mut |_| {}).unwrap(), "done");
    let events = events.borrow();
    let mut last_count = None;
    for event in events.iter() {
        if let Some(payload) = event.strip_prefix("count:") {
            last_count = Some(payload);
        }
        if let Some(payload) = event.strip_prefix("send:") {
            assert_eq!(last_count, Some(payload));
        }
    }
    assert_eq!(events.iter().filter(|e| e.starts_with("send:")).count(), 3);
}

#[test]
fn steering_during_preflight_recounts_new_input_and_stop_never_starts_inference() {
    use atlas_agentic::turns::{TurnControl, TurnManager};
    use std::cell::Cell;
    struct Controlled {
        control: TurnControl,
        counts: Cell<usize>,
        sends: Rc<Cell<usize>>,
        stop: bool,
    }
    impl ModelProvider for Controlled {
        fn complete(&self, _: &ModelRequest) -> Result<String, String> {
            unreachable!()
        }
        fn estimate_request_with_control(
            &self,
            r: &ModelRequest,
            interrupted: &dyn Fn() -> bool,
        ) -> Result<Option<TokenEstimate>, String> {
            self.counts.set(self.counts.get() + 1);
            if self.counts.get() == 1 {
                if self.stop {
                    self.control.cancel();
                } else {
                    self.control.submit("steer", "new instruction").unwrap();
                }
                assert!(
                    interrupted(),
                    "runtime must forward Stop and pending input to the count request"
                );
                return Err("count interrupted".into());
            }
            assert!(r.user_prompt.contains("new instruction"));
            Ok(Some(TokenEstimate {
                tokens: 10,
                estimated: false,
            }))
        }
        fn stream(&self, r: &ModelRequest, _: &mut dyn FnMut(&str)) -> Result<String, String> {
            self.sends.set(self.sends.get() + 1);
            assert!(r.user_prompt.contains("new instruction"));
            Ok("updated answer".into())
        }
    }
    for stop in [false, true] {
        let manager = TurnManager::default();
        let lease = manager.begin("chat").unwrap();
        let sends = Rc::new(Cell::new(0));
        let definition=LocalAgentDefinition::from_toml("name='test'\ndisplay_name='Test'\nmodel_provider='fake'\nmodel='fake'\nsystem_prompt='assistant'\nconversation_title_prompt='title'").unwrap();
        let runtime = Runtime::new(AgentRegistry::new(vec![Box::new(LocalAgent::new(
            definition,
            Box::new(Controlled {
                control: lease.control(),
                counts: Cell::new(0),
                sends: sends.clone(),
                stop,
            }),
        ))]))
        .with_turn_control(lease.control());
        let result = runtime.ask_stream("original", &mut |_| {});
        if stop {
            assert!(result.is_err());
            assert_eq!(sends.get(), 0);
        } else {
            assert_eq!(result.unwrap(), "updated answer");
            assert_eq!(sends.get(), 1);
        }
    }
}
