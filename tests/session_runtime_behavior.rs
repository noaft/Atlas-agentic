use atlas_agentic::{
    agents::{AgentRegistry, LocalAgent, LocalAgentDefinition},
    conversations::{ConversationService, ConversationStore},
    models::{ModelProvider, ModelRequest},
    observability::ObservabilityConfig,
    sessions::SessionStore,
    tools::{Tool, ToolDefinition, ToolRegistry, ToolResult},
    Runtime,
};
use serde_json::json;
use std::{cell::Cell, rc::Rc};
struct Sample(Rc<Cell<usize>>);
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
        self.0.set(self.0.get() + 1);
        Ok(ToolResult::text("tool evidence"))
    }
}
struct Provider {
    calls: Cell<usize>,
    fail: bool,
}
impl ModelProvider for Provider {
    fn complete(&self, _: &ModelRequest) -> Result<String, String> {
        Ok("title".into())
    }
    fn stream(&self, r: &ModelRequest, delta: &mut dyn FnMut(&str)) -> Result<String, String> {
        self.calls.set(self.calls.get() + 1);
        if self.calls.get() == 1 {
            return Ok(r#"{"type":"tool_call","tool":"sample","args":{}}"#.into());
        }
        assert!(r.user_prompt.contains("tool evidence"));
        if self.fail {
            return Err("fake transport failure".into());
        }
        let reply = r#"{"type":"final","content":"done"}"#;
        delta(reply);
        Ok(reply.into())
    }
}
fn runtime(fail: bool, executions: Rc<Cell<usize>>) -> Runtime {
    let def=LocalAgentDefinition::from_toml("name='fake'\ndisplay_name='Fake'\nmodel_provider='fake'\nmodel='fake'\nsystem_prompt='system'\nconversation_title_prompt='title'").unwrap();
    Runtime::new(AgentRegistry::new(vec![Box::new(LocalAgent::new(
        def,
        Box::new(Provider {
            calls: Cell::new(0),
            fail,
        }),
    ))]))
    .with_tools(ToolRegistry::new(vec![Box::new(Sample(executions))]))
}
#[test]
fn failed_turn_keeps_live_steps_and_retry_reuses_session_without_replaying_history() {
    let data = tempfile::tempdir().unwrap();
    let store = ConversationStore::at(data.path().into());
    let executions = Rc::new(Cell::new(0));
    let failed = runtime(true, executions.clone());
    let result = ConversationService::new(&failed, &store, 40)
        .with_session_key("native-draft")
        .ask("first request", None, &mut |_| {});
    assert!(result.is_err());
    assert!(store.list().unwrap().is_empty());
    assert_eq!(executions.get(), 1);
    let sessions = SessionStore::new(data.path(), ObservabilityConfig::default().session).unwrap();
    let journal = sessions
        .open(
            None,
            Some("native-draft"),
            chrono::Utc::now().fixed_offset(),
        )
        .unwrap();
    let id = journal.id();
    let directory = journal.path().to_owned();
    let events = journal.events().unwrap();
    assert!(events.iter().any(|e| e.kind == "tool_call"));
    assert!(events
        .iter()
        .any(|e| e.kind == "turn_context" && e.data["permissionMode"] == "ask"));
    assert!(events
        .iter()
        .any(|e| e.kind == "tool_result" && e.data.to_string().contains("tool evidence")));
    assert!(events.iter().any(|e| e.kind == "context_prepared"));
    assert_eq!(events.last().unwrap().data["status"], "failed");
    drop(journal);
    let successful = runtime(false, executions.clone());
    let result = ConversationService::new(&successful, &store, 40)
        .with_session_key("native-draft")
        .ask("retry request", None, &mut |_| {})
        .unwrap();
    assert_eq!(result.metadata.id, id);
    assert_eq!(executions.get(), 2);
    assert_eq!(sessions.find(&id).unwrap().unwrap(), directory);
    let saved = store.load(&id).unwrap();
    assert_eq!(saved.messages.len(), 2);
    assert_eq!(saved.messages[0].content, "retry request");
    let journal = sessions
        .open(Some(&id), None, chrono::Utc::now().fixed_offset())
        .unwrap();
    let events = journal.events().unwrap();
    assert_eq!(
        events.iter().filter(|e| e.kind == "turn_started").count(),
        2
    );
    assert_eq!(events.iter().filter(|e| e.kind == "thinking").count(), 4);
    assert_eq!(events.last().unwrap().data["status"], "succeeded");
}
#[test]
fn journal_is_created_before_streaming_begins() {
    let data = tempfile::tempdir().unwrap();
    let store = ConversationStore::at(data.path().into());
    let runtime = runtime(false, Rc::new(Cell::new(0)));
    let mut live = false;
    ConversationService::new(&runtime, &store, 40)
        .ask("request", None, &mut |_| {
            // The session must already exist while output is arriving, before chat persistence.
            assert!(data.path().join("sessions/index.json").exists());
            live = true;
        })
        .unwrap();
    assert!(live);
}

#[test]
fn a_journal_write_failure_blocks_tool_execution() {
    use atlas_agentic::{
        conversations::AskCallbacks,
        permissions::{MissingAuthorizer, PermissionMode, ToolEvent},
    };
    let data = tempfile::tempdir().unwrap();
    let store = ConversationStore::at(data.path().into());
    let executions = Rc::new(Cell::new(0));
    let runtime = runtime(false, executions.clone());
    let sessions = SessionStore::new(data.path(), ObservabilityConfig::default().session).unwrap();
    let seed = sessions
        .open(
            None,
            Some("disk-failure"),
            chrono::Utc::now().fixed_offset(),
        )
        .unwrap();
    let events_dir = seed.path().to_owned();
    drop(seed);
    let result = ConversationService::new(&runtime, &store, 40)
        .with_session_key("disk-failure")
        .ask_with_workspace_permission_authorizer_and_fallible_sink(
            "task",
            None,
            None,
            PermissionMode::Ask,
            AskCallbacks {
                authorizer: &MissingAuthorizer,
                on_delta: &mut |_| Ok(()),
                on_tool_event: &mut |event| {
                    if matches!(event, ToolEvent::Requested { .. }) {
                        std::fs::rename(&events_dir, events_dir.with_file_name("old-events"))
                            .unwrap();
                        std::fs::create_dir(&events_dir).unwrap();
                    }
                },
            },
        );
    assert!(result.is_err());
    assert_eq!(executions.get(), 0);
    assert!(store.list().unwrap().is_empty());
}

#[test]
fn committed_history_returns_metadata_with_warning_if_final_journal_writes_fail() {
    use std::sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    };
    let data = tempfile::tempdir().unwrap();
    let sessions = SessionStore::new(data.path(), ObservabilityConfig::default().session).unwrap();
    let journal = sessions
        .open(
            None,
            Some("commit-failure"),
            chrono::Utc::now().fixed_offset(),
        )
        .unwrap();
    let events = journal.path().to_owned();
    drop(journal);
    let changed = Arc::new(AtomicBool::new(false));
    let store = ConversationStore::at_with_index_publisher(
        data.path().into(),
        move |source, destination| {
            std::fs::rename(source, destination)?;
            if !changed.swap(true, Ordering::SeqCst) {
                std::fs::rename(&events, events.with_file_name("old-events"))?;
                std::fs::create_dir(&events)?;
            }
            Ok(())
        },
    );
    let executions = Rc::new(Cell::new(0));
    let runtime = runtime(false, executions.clone());
    let result = ConversationService::new(&runtime, &store, 40)
        .with_session_key("commit-failure")
        .ask("task", None, &mut |_| {})
        .unwrap();
    assert!(result.persistence_warning.is_some());
    assert_eq!(store.load(&result.metadata.id).unwrap().messages.len(), 2);
    assert_eq!(executions.get(), 1);
}

#[test]
fn journal_health_is_checked_before_execution_control_mutations() {
    use atlas_agentic::{
        executions::{protocol::WakeupMode, ToolExecutionManager},
        permissions::{MissingAuthorizer, PermissionMode, ToolEvent},
        runtime::{
            context::{ContextPreparationRequest, PreparedContext, RuntimeContext},
            RuntimeAskCallbacks,
        },
    };
    struct Fixed(String);
    impl ModelProvider for Fixed {
        fn complete(&self, _: &ModelRequest) -> Result<String, String> {
            Ok(self.0.clone())
        }
    }
    struct Checked(Rc<Cell<bool>>);
    impl RuntimeContext for Checked {
        fn check_cancelled(&self) -> Result<(), String> {
            if self.0.get() {
                Err("journal failed".into())
            } else {
                Ok(())
            }
        }
        fn prepare(
            &mut self,
            _: &dyn atlas_agentic::agents::Agent,
            _: ContextPreparationRequest<'_>,
            _: &mut dyn FnMut(&ToolEvent),
        ) -> Result<PreparedContext, String> {
            Ok(PreparedContext {
                memory_blocks: vec![],
                history: vec![],
                activity_context: String::new(),
                max_output_tokens: None,
            })
        }
    }
    let manager = ToolExecutionManager::new();
    let scope = manager.scope("owner");
    let info = scope
        .start("seed", |_| Ok(ToolResult::text("done")))
        .unwrap();
    scope
        .wait(std::slice::from_ref(&info.execution_id), 100)
        .unwrap();
    let response=json!({"type":"set_execution_wakeup","execution_id":info.execution_id,"wake_up":{"mode":"lazy"}}).to_string();
    let def=LocalAgentDefinition::from_toml("name='fake'\ndisplay_name='Fake'\nmodel_provider='fake'\nmodel='fake'\nsystem_prompt='system'\nconversation_title_prompt='title'").unwrap();
    let runtime = Runtime::new(AgentRegistry::new(vec![Box::new(LocalAgent::new(
        def,
        Box::new(Fixed(response)),
    ))]))
    .with_execution_scope(scope.clone());
    let failed = Rc::new(Cell::new(false));
    let mut context = Checked(failed.clone());
    assert!(runtime
        .ask_stream_with_managed_context(
            "task",
            &mut context,
            PermissionMode::Ask,
            RuntimeAskCallbacks {
                authorizer: &MissingAuthorizer,
                on_delta: &mut |_| {},
                on_tool_event: &mut |event| {
                    if matches!(event,ToolEvent::Started{tool,..} if tool=="set_execution_wakeup") {
                        failed.set(true);
                    }
                }
            }
        )
        .is_err());
    assert_eq!(
        scope.info(&info.execution_id).unwrap().wake_up.mode,
        WakeupMode::Hard
    );
}

#[test]
fn deleting_a_conversation_archives_its_session_and_rolls_both_back_on_index_failure() {
    let data = tempfile::tempdir().unwrap();
    let store = ConversationStore::at(data.path().into());
    let runtime = runtime(false, Rc::new(Cell::new(0)));
    let result = ConversationService::new(&runtime, &store, 40)
        .ask("task", None, &mut |_| {})
        .unwrap();
    let sessions = SessionStore::new(data.path(), ObservabilityConfig::default().session).unwrap();
    let directory = sessions.find(&result.metadata.id).unwrap().unwrap();
    let failing = ConversationStore::at_with_index_publisher(data.path().into(), |_, _| {
        Err(std::io::Error::other("injected publication failure"))
    });
    assert!(failing.delete(&result.metadata.id).is_err());
    assert!(store.load(&result.metadata.id).is_ok());
    assert_eq!(
        sessions.find(&result.metadata.id).unwrap().unwrap(),
        directory
    );
    assert!(directory.exists());
    store.delete(&result.metadata.id).unwrap();
    assert!(!directory.exists());
    assert!(sessions.find(&result.metadata.id).unwrap().is_none());
    assert!(data
        .path()
        .join("trash")
        .join(format!("{}.session", result.metadata.id))
        .is_file());
}
