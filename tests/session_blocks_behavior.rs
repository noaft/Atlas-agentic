use atlas_agentic::{
    agents::{AgentRegistry, LocalAgent, LocalAgentDefinition},
    conversations::{ConversationId, ConversationService, ConversationStore},
    models::{ModelProvider, ModelRequest},
    observability::ObservabilityConfig,
    sessions::SessionStore,
    tools::{Tool, ToolDefinition, ToolRegistry, ToolResult},
    Runtime,
};
use serde_json::{json, Value};
use std::{
    cell::Cell,
    fs,
    path::{Path, PathBuf},
};

fn session_path(root: &Path) -> PathBuf {
    let index: Value =
        serde_json::from_slice(&fs::read(root.join("sessions/index.json")).unwrap()).unwrap();
    let id = ConversationId::parse(
        index["sessions"]
            .as_object()
            .unwrap()
            .keys()
            .next()
            .unwrap(),
    )
    .unwrap();
    SessionStore::new(root, ObservabilityConfig::default().session)
        .unwrap()
        .find(&id)
        .unwrap()
        .unwrap()
}
fn events(root: &Path) -> Vec<Value> {
    let document: Value = serde_json::from_slice(&fs::read(session_path(root)).unwrap()).unwrap();
    document["events"].as_array().unwrap().clone()
}
struct Script {
    calls: Cell<usize>,
    replies: Vec<(String, bool)>,
    unchanged_during_stream: Option<PathBuf>,
}
impl ModelProvider for Script {
    fn complete(&self, _: &ModelRequest) -> Result<String, String> {
        Ok("Title".into())
    }
    fn stream(&self, _: &ModelRequest, delta: &mut dyn FnMut(&str)) -> Result<String, String> {
        let n = self.calls.get();
        self.calls.set(n + 1);
        let (reply, fails) = &self.replies[n];
        let before = self
            .unchanged_during_stream
            .as_ref()
            .map(|root| fs::read(session_path(root)).unwrap());
        for character in reply.chars() {
            delta(&character.to_string());
            if let Some(root) = &self.unchanged_during_stream {
                assert!(
                    fs::read(session_path(root)).unwrap() == *before.as_ref().unwrap(),
                    "stream chunks and previews must not rewrite the session"
                );
            }
        }
        if *fails {
            Err("fake transport failure".into())
        } else {
            Ok(reply.clone())
        }
    }
}
struct Evidence(PathBuf);
impl Tool for Evidence {
    fn name(&self) -> &str {
        "evidence"
    }
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "evidence".into(),
            description: "Read evidence".into(),
            input_schema: json!({"type":"object"}),
        }
    }
    fn execute(&self, _: &Value) -> Result<ToolResult, String> {
        assert!(
            events(&self.0).iter().any(|e| e["kind"] == "tool_call"),
            "call must be durable before execution"
        );
        Ok(ToolResult::text("evidence found"))
    }
}
fn runtime(root: &Path, replies: Vec<(String, bool)>, stable: bool) -> Runtime {
    let def = LocalAgentDefinition::from_toml("name='fake'\ndisplay_name='Fake'\nmodel_provider='fake'\nmodel='fake'\nsystem_prompt='system'\nconversation_title_prompt='title'").unwrap();
    Runtime::new(AgentRegistry::new(vec![Box::new(LocalAgent::new(
        def,
        Box::new(Script {
            calls: Cell::new(0),
            replies,
            unchanged_during_stream: stable.then(|| root.to_owned()),
        }),
    ))]))
    .with_tools(ToolRegistry::new(vec![Box::new(Evidence(root.to_owned()))]))
}
fn final_reply(content: &str) -> (String, bool) {
    (
        format!(r#"{{"type":"final","content":{}}}"#, json!(content)),
        false,
    )
}

#[test]
fn streamed_answer_and_progress_are_published_only_as_completed_blocks() {
    let data = tempfile::tempdir().unwrap();
    let store = ConversationStore::at(data.path().into());
    let runtime = runtime(
        data.path(),
        vec![
            (
                r#"{"type":"tool_call","message":"Checking evidence","tool":"evidence","args":{}}"#
                    .into(),
                false,
            ),
            final_reply("Kết quả đầy đủ"),
        ],
        true,
    );
    let mut displayed = String::new();
    let result = ConversationService::new(&runtime, &store, 40)
        .ask("task", None, &mut |s| displayed.push_str(s))
        .unwrap();
    assert_eq!(displayed, "Kết quả đầy đủ");
    assert_eq!(result.answer, displayed);
    let events = events(data.path());
    let relevant: Vec<_> = events
        .iter()
        .filter_map(|e| e["kind"].as_str())
        .filter(|kind| {
            [
                "thinking",
                "progress",
                "tool_call",
                "tool_result",
                "assistant",
            ]
            .contains(kind)
        })
        .collect();
    assert_eq!(
        relevant,
        [
            "thinking",
            "tool_call",
            "progress",
            "tool_result",
            "thinking",
            "assistant"
        ]
    );
    assert!(!events.iter().any(|e| [
        "assistant_delta",
        "assistant_final",
        "progress_preview",
        "model_state",
        "runtime_status",
        "model_usage",
        "model_call_started",
        "model_call_finished"
    ]
    .contains(&e["kind"].as_str().unwrap())));
    let answer = events.iter().find(|e| e["kind"] == "assistant").unwrap();
    assert_eq!(answer["data"]["content"], displayed);
    assert_eq!(answer["data"]["status"], "succeeded");
    let thinking = events.iter().find(|e| e["kind"] == "thinking").unwrap();
    assert!(thinking["data"]["durationMs"].is_u64());
    assert!(thinking["data"]["startedAt"].is_string());
    assert_eq!(thinking["data"]["status"], "succeeded");
}

#[test]
fn transport_failure_seals_partial_output_without_committing_chat_history() {
    let data = tempfile::tempdir().unwrap();
    let store = ConversationStore::at(data.path().into());
    let runtime = runtime(
        data.path(),
        vec![(r#"{"type":"final","content":"partial"#.into(), true)],
        false,
    );
    assert!(ConversationService::new(&runtime, &store, 40)
        .ask("task", None, &mut |_| {})
        .is_err());
    assert!(store.list().unwrap().is_empty());
    let events = events(data.path());
    let answers: Vec<_> = events.iter().filter(|e| e["kind"] == "assistant").collect();
    assert_eq!(answers.len(), 1);
    assert_eq!(answers[0]["data"]["content"], "partial");
    assert_eq!(answers[0]["data"]["status"], "failed");
    assert_eq!(
        events.iter().find(|e| e["kind"] == "thinking").unwrap()["data"]["status"],
        "failed"
    );
}

#[test]
fn protocol_reset_separates_discarded_partial_from_the_completed_answer() {
    let data = tempfile::tempdir().unwrap();
    let store = ConversationStore::at(data.path().into());
    let runtime = runtime(
        data.path(),
        vec![
            (
                r#"{"type":"final","content":"stale","broken":"#.into(),
                false,
            ),
            final_reply("corrected"),
        ],
        false,
    );
    let result = ConversationService::new(&runtime, &store, 40)
        .ask("task", None, &mut |_| {})
        .unwrap();
    assert_eq!(result.answer, "corrected");
    let events = events(data.path());
    let answers: Vec<_> = events.iter().filter(|e| e["kind"] == "assistant").collect();
    assert_eq!(answers.len(), 2);
    assert_eq!(answers[0]["data"]["content"], "stale");
    assert_eq!(answers[0]["data"]["status"], "superseded");
    assert_eq!(answers[1]["data"]["content"], "corrected");
    assert_eq!(answers[1]["data"]["status"], "succeeded");
    let reset = events
        .iter()
        .find(|e| e["kind"] == "response_reset")
        .unwrap();
    assert!(answers[0]["sequence"].as_u64().unwrap() < reset["sequence"].as_u64().unwrap());
    assert_eq!(
        store.load(&result.metadata.id).unwrap().messages[1].content,
        "corrected"
    );
}

#[test]
fn explicit_stop_seals_only_the_authoritative_displayed_prefix() {
    use atlas_agentic::turns::{DisplayCursor, TurnManager};
    let data = tempfile::tempdir().unwrap();
    let store = ConversationStore::at(data.path().into());
    let turns = TurnManager::default();
    let lease = turns.begin("chat").unwrap();
    let control = lease.control();
    let cancel = control.cancelled();
    let runtime =
        runtime(data.path(), vec![final_reply("hello")], false).with_turn_control(control.clone());
    let mut received = 0;
    let result = ConversationService::new(&runtime, &store, 40)
        .with_cancellation(&cancel)
        .ask("task", None, &mut |s| {
            received += s.len();
            if received >= 2 {
                turns
                    .cancel_with_cursor(
                        "chat",
                        control.request_id(),
                        Some(DisplayCursor {
                            response_epoch: control.response_epoch(),
                            displayed_bytes: 1,
                        }),
                    )
                    .unwrap();
            }
        })
        .unwrap();
    assert!(result.interrupted);
    assert_eq!(result.answer, "h");
    let events = events(data.path());
    let answers: Vec<_> = events.iter().filter(|e| e["kind"] == "assistant").collect();
    assert_eq!(answers.len(), 1);
    assert_eq!(answers[0]["data"]["content"], "h");
    assert_eq!(answers[0]["data"]["status"], "stopped");
    assert_eq!(
        store.load(&result.metadata.id).unwrap().messages[1].content,
        "h"
    );
}

#[test]
fn failed_display_sink_seals_only_accepted_text_and_does_not_commit_history() {
    use atlas_agentic::{
        conversations::AskCallbacks,
        permissions::{MissingAuthorizer, PermissionMode},
    };
    let data = tempfile::tempdir().unwrap();
    let store = ConversationStore::at(data.path().into());
    let runtime = runtime(data.path(), vec![final_reply("hello")], false);
    let mut received = 0;
    let result = ConversationService::new(&runtime, &store, 40)
        .ask_with_workspace_permission_authorizer_and_fallible_sink(
            "task",
            None,
            None,
            PermissionMode::Ask,
            AskCallbacks {
                authorizer: &MissingAuthorizer,
                on_tool_event: &mut |_| {},
                on_delta: &mut |_| {
                    received += 1;
                    if received > 1 {
                        Err("sink disconnected".into())
                    } else {
                        Ok(())
                    }
                },
            },
        );
    assert!(result.is_err());
    assert!(store.list().unwrap().is_empty());
    let events = events(data.path());
    let answers: Vec<_> = events.iter().filter(|e| e["kind"] == "assistant").collect();
    assert_eq!(answers.len(), 1);
    assert_eq!(answers[0]["data"]["content"], "h");
    assert_eq!(answers[0]["data"]["status"], "failed");
}
