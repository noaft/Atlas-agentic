use atlas_agentic::turns::TurnManager;

#[test]
fn mailbox_is_ordered_idempotent_and_sealed_atomically() {
    let manager = TurnManager::default();
    let lease = manager.begin("chat").unwrap();
    assert!(manager.begin("chat").is_err());
    let control = lease.control();
    assert!(control.submit("one", "first").unwrap().accepted);
    assert!(control.submit("one", "first").unwrap().accepted);
    assert!(control.submit("one", "conflicting").is_err());
    control.submit("two", "second").unwrap();
    assert!(!control.try_finish());
    let inputs = control.drain();
    assert_eq!(
        inputs
            .iter()
            .map(|i| i.content.as_str())
            .collect::<Vec<_>>(),
        ["first", "second"]
    );
    assert_eq!(control.applied(), inputs);
    assert!(control.try_finish());
    let late = control.submit("three", "late").unwrap();
    assert!(late.finished);
    assert!(!late.accepted);
    drop(lease);
    assert!(manager.begin("chat").is_ok());
}

#[test]
fn owner_and_request_tokens_are_required_and_drop_closes_ingress() {
    let manager = TurnManager::default();
    let lease = manager.begin("chat").unwrap();
    let control = lease.control();
    assert!(manager
        .submit("other", control.request_id(), "one", "x")
        .is_err());
    assert!(manager.submit("chat", "wrong", "one", "x").is_err());
    manager.cancel("chat", control.request_id()).unwrap();
    assert!(control.check().is_err());
    drop(lease);
    assert!(control.submit("two", "x").unwrap().finished);
}

#[test]
fn mailbox_bounds_include_already_applied_messages() {
    let manager = TurnManager::default();
    let lease = manager.begin("chat").unwrap();
    let control = lease.control();
    assert!(control.submit("large", &"x".repeat(65537)).is_err());
    for index in 0..32 {
        control.submit(&index.to_string(), "x").unwrap();
        control.drain();
    }
    assert!(control.submit("overflow", "x").is_err());
}

use atlas_agentic::{
    agents::{Agent, AgentRegistry},
    conversations::ConversationMessage,
    runtime::Runtime,
    tools::ToolDefinition,
};
use std::{
    cell::Cell,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};
struct SteeringAgent {
    control: atlas_agentic::turns::TurnControl,
    calls: Cell<usize>,
    cancel: bool,
}
impl Agent for SteeringAgent {
    fn name(&self) -> &str {
        "test"
    }
    fn system_prompt(&self) -> &str {
        ""
    }
    fn process(
        &self,
        _: &str,
        _: &[ConversationMessage],
        _: &[ToolDefinition],
    ) -> Result<String, String> {
        unreachable!()
    }
    fn generate_conversation_title(&self, _: &str) -> Result<String, String> {
        Ok("test".into())
    }
    fn process_stream(
        &self,
        input: &str,
        _: &[ConversationMessage],
        _: &[ToolDefinition],
        delta: &mut dyn FnMut(&str),
    ) -> Result<String, String> {
        self.calls.set(self.calls.get() + 1);
        if self.calls.get() == 1 {
            delta("stale");
            if self.cancel {
                self.control.cancel();
                delta("hidden");
            } else {
                self.control.submit("steer", "correct it").unwrap();
            }
            Ok("stale".into())
        } else {
            let input: serde_json::Value = serde_json::from_str(input).unwrap();
            assert_eq!(input["latest_user_messages"][0]["content"], "correct it");
            assert_eq!(input["original_user_request"], "original");
            delta("updated");
            Ok("updated".into())
        }
    }
}
#[test]
fn steering_discards_stale_final_and_preserves_typed_user_message() {
    let manager = TurnManager::default();
    let lease = manager.begin("chat").unwrap();
    let control = lease.control();
    let runtime = Runtime::new(AgentRegistry::new(vec![Box::new(SteeringAgent {
        control: control.clone(),
        calls: Cell::new(0),
        cancel: false,
    })]))
    .with_turn_control(control.clone());
    let mut resets = 0;
    let mut accepted = 0;
    let answer = runtime
        .ask_stream_with_history_and_authorization(
            "original",
            &[],
            atlas_agentic::permissions::PermissionMode::Ask,
            &atlas_agentic::permissions::MissingAuthorizer,
            &mut |_| {},
            &mut |event| match event {
                atlas_agentic::permissions::ToolEvent::ResponseReset => resets += 1,
                atlas_agentic::permissions::ToolEvent::UserInputAccepted { .. } => accepted += 1,
                _ => {}
            },
        )
        .unwrap();
    assert_eq!(answer, "updated");
    assert_eq!(resets, 1);
    assert_eq!(accepted, 1);
    assert_eq!(runtime.applied_user_inputs().len(), 1);
    assert!(control.submit("late", "next").unwrap().finished);
}
#[test]
fn cancellation_suppresses_later_deltas_and_uses_shared_native_flag() {
    let manager = TurnManager::default();
    let flag = Arc::new(AtomicBool::new(false));
    let lease = manager
        .begin_with_cancellation("chat", flag.clone())
        .unwrap();
    let runtime = Runtime::new(AgentRegistry::new(vec![Box::new(SteeringAgent {
        control: lease.control(),
        calls: Cell::new(0),
        cancel: true,
    })]))
    .with_turn_control(lease.control());
    let mut text = String::new();
    assert!(runtime
        .ask_stream("original", &mut |s| text.push_str(s))
        .is_err());
    assert_eq!(text, "stale");
    assert!(flag.load(Ordering::Acquire));
}

#[test]
fn scheduler_interrupts_lazy_wait_without_fabricating_completion() {
    use atlas_agentic::executions::{
        protocol::{ExecutionPolicy, WakeupMode},
        ToolExecutionManager,
    };
    let manager = ToolExecutionManager::new();
    let scope = manager.scope("owner");
    let (tx, rx) = std::sync::mpsc::channel();
    let run = scope
        .start("held", move |_| {
            rx.recv().unwrap();
            Ok(atlas_agentic::tools::ToolResult::text("done"))
        })
        .unwrap();
    scope
        .set_wakeup(
            &run.execution_id,
            ExecutionPolicy {
                mode: WakeupMode::Lazy,
                interval_seconds: None,
            },
        )
        .unwrap();
    let result = scope
        .wait_scheduled_interruptible_checked(std::slice::from_ref(&run.execution_id), || Ok(true))
        .unwrap();
    assert!(result.is_none());
    assert!(scope.claim().unwrap().is_none());
    tx.send(()).unwrap();
}

#[test]
fn steering_withdraws_obsolete_approval_even_when_authorizer_returns_error() {
    use atlas_agentic::{
        permissions::*,
        tools::{Tool, ToolRegistry, ToolResult},
    };
    struct Dangerous;
    impl Tool for Dangerous {
        fn name(&self) -> &str {
            "danger"
        }
        fn definition(&self) -> ToolDefinition {
            ToolDefinition {
                name: "danger".into(),
                description: "test".into(),
                input_schema: serde_json::json!({"type":"object"}),
            }
        }
        fn risk(&self, _: &serde_json::Value) -> Result<ToolRisk, String> {
            Ok(ToolRisk::Command)
        }
        fn execute(&self, _: &serde_json::Value) -> Result<ToolResult, String> {
            panic!("superseded action executed")
        }
    }
    struct Script(Cell<usize>);
    impl Agent for Script {
        fn name(&self) -> &str {
            "script"
        }
        fn system_prompt(&self) -> &str {
            ""
        }
        fn generate_conversation_title(&self, _: &str) -> Result<String, String> {
            unreachable!()
        }
        fn process(
            &self,
            _: &str,
            _: &[ConversationMessage],
            _: &[ToolDefinition],
        ) -> Result<String, String> {
            unreachable!()
        }
        fn process_stream(
            &self,
            input: &str,
            _: &[ConversationMessage],
            _: &[ToolDefinition],
            _: &mut dyn FnMut(&str),
        ) -> Result<String, String> {
            self.0.set(self.0.get() + 1);
            if self.0.get() == 1 {
                Ok(r#"{"type":"tool_call","tool":"danger","args":{}}"#.into())
            } else {
                assert!(input.contains("correction"));
                Ok(r#"{"type":"final","content":"updated"}"#.into())
            }
        }
    }
    struct Steer(atlas_agentic::turns::TurnControl);
    impl ToolAuthorizer for Steer {
        fn authorize(&self, _: &AuthorizationRequest) -> Result<AuthorizationDecision, String> {
            self.0.submit("change", "correction").unwrap();
            Err("Request updated".into())
        }
    }
    let manager = TurnManager::default();
    let lease = manager.begin("chat").unwrap();
    let mut tools = ToolRegistry::empty();
    tools.register(Box::new(Dangerous));
    let runtime = Runtime::new(AgentRegistry::new(vec![Box::new(Script(Cell::new(0)))]))
        .with_tools(tools)
        .with_turn_control(lease.control());
    let result = runtime
        .ask_stream_with_history_and_authorization(
            "original",
            &[],
            PermissionMode::Ask,
            &Steer(lease.control()),
            &mut |_| {},
            &mut |_| {},
        )
        .unwrap();
    assert_eq!(result, "updated");
}

#[test]
fn finished_control_never_starts_another_model_call() {
    let manager = TurnManager::default();
    let lease = manager.begin("chat").unwrap();
    let control = lease.control();
    assert!(control.try_finish());
    let runtime = Runtime::new(AgentRegistry::new(vec![Box::new(SteeringAgent {
        control: control.clone(),
        calls: Cell::new(0),
        cancel: false,
    })]))
    .with_turn_control(control);
    assert!(runtime.ask_stream("must not run", &mut |_| {}).is_err());
}

#[test]
fn explicit_stop_is_distinct_from_owner_teardown() {
    let manager = TurnManager::default();
    let first = manager.begin("one").unwrap();
    let second = manager.begin("two").unwrap();
    manager.cancel("one", first.control().request_id()).unwrap();
    manager.close_owner("two");
    assert!(first.control().user_stop_requested());
    assert!(!second.control().user_stop_requested());
    assert!(second.control().check().is_err());
}

#[test]
fn live_input_releases_runtime_await_without_waiting_for_lazy_tool() {
    use atlas_agentic::{
        executions::{
            protocol::{ExecutionPolicy, WakeupMode},
            ToolExecutionManager,
        },
        permissions::*,
    };
    struct WaitingAgent {
        id: String,
        calls: Cell<usize>,
    }
    impl Agent for WaitingAgent {
        fn name(&self) -> &str {
            "wait"
        }
        fn system_prompt(&self) -> &str {
            ""
        }
        fn generate_conversation_title(&self, _: &str) -> Result<String, String> {
            unreachable!()
        }
        fn process(
            &self,
            _: &str,
            _: &[ConversationMessage],
            _: &[ToolDefinition],
        ) -> Result<String, String> {
            unreachable!()
        }
        fn process_stream(
            &self,
            input: &str,
            _: &[ConversationMessage],
            _: &[ToolDefinition],
            _: &mut dyn FnMut(&str),
        ) -> Result<String, String> {
            self.calls.set(self.calls.get() + 1);
            if self.calls.get() == 1 {
                Ok(
                    serde_json::json!({"type":"await_execution","execution_ids":[self.id]})
                        .to_string(),
                )
            } else {
                assert!(input.contains("new direction"));
                Ok(r#"{"type":"final","content":"changed"}"#.into())
            }
        }
    }
    let manager = TurnManager::default();
    let lease = manager.begin("owner").unwrap();
    let executions = ToolExecutionManager::new();
    let scope = executions.scope("owner");
    let (tx, rx) = std::sync::mpsc::channel();
    let run = scope
        .start("held", move |_| {
            rx.recv().unwrap();
            Ok(atlas_agentic::tools::ToolResult::text("done"))
        })
        .unwrap();
    scope
        .set_wakeup(
            &run.execution_id,
            ExecutionPolicy {
                mode: WakeupMode::Lazy,
                interval_seconds: None,
            },
        )
        .unwrap();
    let runtime = Runtime::new(AgentRegistry::new(vec![Box::new(WaitingAgent {
        id: run.execution_id,
        calls: Cell::new(0),
    })]))
    .with_execution_scope(scope)
    .with_turn_control(lease.control());
    let answer = runtime
        .ask_stream_with_history_and_authorization(
            "wait",
            &[],
            PermissionMode::Ask,
            &MissingAuthorizer,
            &mut |_| {},
            &mut |event| {
                if matches!(event,ToolEvent::Started{tool,..} if tool=="await_execution") {
                    lease.control().submit("steer", "new direction").unwrap();
                }
            },
        )
        .unwrap();
    assert_eq!(answer, "changed");
    tx.send(()).unwrap();
}

#[test]
fn concurrent_final_seal_never_loses_an_accepted_message() {
    for _ in 0..64 {
        let manager = TurnManager::default();
        let lease = manager.begin("race").unwrap();
        let control = lease.control();
        let sender = control.clone();
        let gate = Arc::new(std::sync::Barrier::new(2));
        let other = gate.clone();
        let thread = std::thread::spawn(move || {
            other.wait();
            sender.submit("race", "last instruction").unwrap()
        });
        gate.wait();
        let finished = control.try_finish();
        let receipt = thread.join().unwrap();
        assert_eq!(finished, !receipt.accepted);
        if receipt.accepted {
            assert_eq!(control.drain()[0].content, "last instruction");
        }
    }
}

#[test]
fn mailbox_aggregate_limit_counts_utf8_bytes() {
    let manager = TurnManager::default();
    let lease = manager.begin("bounded").unwrap();
    for index in 0..4 {
        lease
            .control()
            .submit(&index.to_string(), &"é".repeat(32768))
            .unwrap();
    }
    assert!(lease.control().submit("too much", "x").is_err());
}

#[test]
fn managed_context_marks_interrupted_assistant_without_changing_stored_history() {
    use atlas_agentic::{
        conversations::*,
        runtime::context::{ContextPreparationRequest, ConversationContext, RuntimeContext},
    };
    let dir = tempfile::tempdir().unwrap();
    let store = ConversationStore::at(dir.path().into());
    let mut activity = ActivityCollector::new().into_unassociated_turn();
    activity.interrupted = true;
    let metadata = store
        .create_turn_with_activity("question", "partial", None, None, activity)
        .unwrap();
    let saved = store.load(&metadata.id).unwrap();
    let mut context = ConversationContext {
        store: &store,
        conversation: Some(saved),
        policy: ContextPolicy::default(),
        cancelled: None,
    };
    let manager = TurnManager::default();
    let lease = manager.begin("context").unwrap();
    let agent = SteeringAgent {
        control: lease.control(),
        calls: Cell::new(0),
        cancel: false,
    };
    let prepared = context
        .prepare(
            &agent,
            ContextPreparationRequest {
                input: "continue",
                tools: &[],
                images: &[],
                latest_usage: None,
            },
            &mut |_| {},
        )
        .unwrap();
    assert!(prepared.history[1].content.contains("incomplete"));
    assert_eq!(context.conversation.unwrap().messages[1].content, "partial");
    assert_eq!(
        store.load(&metadata.id).unwrap().messages[1].content,
        "partial"
    );
}

#[test]
fn teardown_overrides_prior_explicit_stop() {
    let manager = TurnManager::default();
    let lease = manager.begin("chat").unwrap();
    manager
        .cancel("chat", lease.control().request_id())
        .unwrap();
    manager.close_owner("chat");
    assert!(!lease.control().user_stop_requested());
}

#[test]
fn stop_cursor_keeps_visible_epoch_prefix_and_excludes_late_sent_bytes() {
    use atlas_agentic::turns::DisplayCursor;
    let manager = TurnManager::default();
    let lease = manager.begin("chat").unwrap();
    let control = lease.control();
    control.record_displayed_delta("old visible");
    let epoch = control.response_epoch();
    control.reset_displayed_output();
    control.record_displayed_delta("unseen replacement");
    manager
        .cancel_with_cursor(
            "chat",
            control.request_id(),
            Some(DisplayCursor {
                response_epoch: epoch,
                displayed_bytes: 3,
            }),
        )
        .unwrap();
    control.record_displayed_delta("late unseen");
    assert_eq!(control.stopped_prefix().unwrap(), Some("old".into()));
    manager
        .cancel_with_cursor("chat", control.request_id(), None)
        .unwrap();
    assert_eq!(control.stopped_prefix().unwrap(), Some("old".into()));
}

#[test]
fn stopped_prefix_validates_after_inflight_sent_chunk_is_recorded() {
    use atlas_agentic::turns::DisplayCursor;
    let manager = TurnManager::default();
    let lease = manager.begin("chat").unwrap();
    let control = lease.control();
    manager
        .cancel_with_cursor(
            "chat",
            control.request_id(),
            Some(DisplayCursor {
                response_epoch: 0,
                displayed_bytes: 2,
            }),
        )
        .unwrap();
    assert!(control.stopped_prefix().is_err());
    control.record_displayed_delta("éhidden");
    assert_eq!(control.stopped_prefix().unwrap(), Some("é".into()));
    let other = manager.begin("other").unwrap();
    other.control().record_displayed_delta("é");
    manager
        .cancel_with_cursor(
            "other",
            other.control().request_id(),
            Some(DisplayCursor {
                response_epoch: 0,
                displayed_bytes: 1,
            }),
        )
        .unwrap();
    assert!(other.control().stopped_prefix().is_err());
}

#[test]
fn evicted_display_epoch_fails_closed() {
    use atlas_agentic::turns::DisplayCursor;
    let manager = TurnManager::default();
    let lease = manager.begin("chat").unwrap();
    let control = lease.control();
    control.record_displayed_delta("old");
    for _ in 0..64 {
        control.reset_displayed_output();
    }
    manager
        .cancel_with_cursor(
            "chat",
            control.request_id(),
            Some(DisplayCursor {
                response_epoch: 0,
                displayed_bytes: 1,
            }),
        )
        .unwrap();
    assert!(control.stopped_prefix().is_err());
}

#[test]
fn oversized_native_display_eviction_never_reconstructs_a_suffix_as_prefix() {
    use atlas_agentic::turns::DisplayCursor;
    let manager = TurnManager::default();
    let lease = manager.begin("chat").unwrap();
    let control = lease.control();
    control.record_displayed_delta(&"x".repeat(16 * 1024 * 1024));
    control.record_displayed_delta("overflow");
    control.record_displayed_delta("later");
    manager
        .cancel_with_cursor(
            "chat",
            control.request_id(),
            Some(DisplayCursor {
                response_epoch: 0,
                displayed_bytes: 1,
            }),
        )
        .unwrap();
    assert!(control.stopped_prefix().is_err());
}

#[test]
fn display_cursor_rejects_future_epochs_and_metadata_overrun() {
    use atlas_agentic::turns::DisplayCursor;
    let manager = TurnManager::default();
    let lease = manager.begin("chat").unwrap();
    let control = lease.control();
    assert!(manager
        .cancel_with_cursor(
            "chat",
            control.request_id(),
            Some(DisplayCursor {
                response_epoch: 1,
                displayed_bytes: 0
            })
        )
        .is_err());
    assert!(manager
        .cancel_with_cursor(
            "chat",
            control.request_id(),
            Some(DisplayCursor {
                response_epoch: 0,
                displayed_bytes: 16 * 1024 * 1024 + 1
            })
        )
        .is_err());
    assert!(control.check().is_ok());
    manager.cancel("chat", control.request_id()).unwrap();
    manager
        .cancel_with_cursor(
            "chat",
            control.request_id(),
            Some(DisplayCursor {
                response_epoch: 0,
                displayed_bytes: 0,
            }),
        )
        .unwrap();
    assert_eq!(control.stopped_prefix().unwrap(), None);
}

#[test]
fn steering_interrupts_inference_and_restarts_with_the_new_input() {
    struct InterruptibleAgent {
        control: atlas_agentic::turns::TurnControl,
        calls: Cell<usize>,
    }
    impl atlas_agentic::models::ModelProvider for InterruptibleAgent {
        fn complete(&self, _: &atlas_agentic::models::ModelRequest) -> Result<String, String> {
            Err("uninterruptible inference entrypoint used".into())
        }
        fn stream_with_control(
            &self,
            request: &atlas_agentic::models::ModelRequest,
            delta: &mut dyn FnMut(&str),
            interrupted: &dyn Fn() -> bool,
        ) -> Result<atlas_agentic::models::ModelResponse, String> {
            self.calls.set(self.calls.get() + 1);
            if self.calls.get() == 1 {
                delta("old prefix");
                self.control.submit("steer", "change direction").unwrap();
                assert!(interrupted(), "Steer did not cancel inference");
                delta("must be suppressed");
                return Err("Inference interrupted".into());
            }
            assert!(!interrupted());
            let input: serde_json::Value = serde_json::from_str(request.user_prompt).unwrap();
            assert_eq!(
                input["latest_user_messages"][0]["content"],
                "change direction"
            );
            delta("new answer");
            Ok(atlas_agentic::models::ModelResponse {
                content: "new answer".into(),
                usage: None,
            })
        }
    }
    let manager = TurnManager::default();
    let lease = manager.begin("owner").unwrap();
    let definition = atlas_agentic::agents::LocalAgentDefinition::from_toml(
        "name='test'\ndisplay_name='Test'\nmodel_provider='deepseek'\nmodel='fake'\nsystem_prompt='System'\nconversation_title_prompt='Title'\n",
    ).unwrap();
    let agent = atlas_agentic::agents::LocalAgent::new(
        definition,
        Box::new(InterruptibleAgent {
            control: lease.control(),
            calls: Cell::new(0),
        }),
    );
    let runtime =
        Runtime::new(AgentRegistry::new(vec![Box::new(agent)])).with_turn_control(lease.control());
    let mut shown = String::new();
    let mut resets = 0;
    let result = runtime.ask_stream_with_history_and_authorization(
        "original",
        &[],
        atlas_agentic::permissions::PermissionMode::Ask,
        &atlas_agentic::permissions::MissingAuthorizer,
        &mut |delta| shown.push_str(delta),
        &mut |event| {
            if matches!(event, atlas_agentic::permissions::ToolEvent::ResponseReset) {
                resets += 1;
            }
        },
    );
    assert_eq!(result.unwrap(), "new answer");
    assert_eq!(shown, "old prefixnew answer");
    assert_eq!(resets, 1);
}
