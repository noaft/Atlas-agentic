use atlas_agentic::{
    conversations::ConversationId,
    tasks::{TaskRegistry, TaskRequest},
};
use serde_json::json;

#[test]
fn catalog_is_available_without_model_configuration_and_validates_requests() {
    let registry = TaskRegistry::builtins();
    let tasks = registry.descriptors();
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].command, "compact");
    let mut request = TaskRequest {
        task_id: "conversation.compact".into(),
        arguments: json!({}),
        conversation_id: None,
    };
    assert!(registry
        .validate(&request)
        .unwrap_err()
        .contains("conversation"));
    request.conversation_id = Some(ConversationId::new());
    assert!(registry.validate(&request).is_ok());
    for args in [json!(null), json!([]), json!({"unexpected": true})] {
        request.arguments = args;
        assert!(registry
            .validate(&request)
            .unwrap_err()
            .contains("arguments"));
    }
    request.task_id = "unknown".into();
    assert!(registry
        .validate(&request)
        .unwrap_err()
        .contains("Unknown task"));
}

#[test]
fn registry_rejects_duplicate_ids_and_case_insensitive_commands() {
    let task = atlas_agentic::tasks::TaskDefinition {
        descriptor: TaskRegistry::builtins().descriptors().remove(0),
        handler: atlas_agentic::tasks::TaskHandler::Compact,
    };
    assert!(TaskRegistry::new(vec![task.clone(), task.clone()]).is_err());
    let mut other = task.clone();
    other.descriptor.id = "another".into();
    other.descriptor.command = "COMPACT".into();
    assert!(TaskRegistry::new(vec![task, other]).is_err());
}

#[test]
fn dispatch_compacts_without_history_changes_and_respects_the_execution_lease() {
    use atlas_agentic::{
        agents::{AgentRegistry, LocalAgent, LocalAgentDefinition},
        conversations::{ConversationService, ConversationStore},
        models::*,
        tasks::TaskOutcome,
        Runtime,
    };
    struct Model;
    impl ModelProvider for Model {
        fn complete(&self, _: &ModelRequest) -> Result<String, String> {
            panic!("no chat/title calls")
        }
        fn stream(&self, _: &ModelRequest, _: &mut dyn FnMut(&str)) -> Result<String, String> {
            panic!("no chat calls")
        }
        fn model_limits(&self) -> Result<Option<ModelLimits>, String> {
            Ok(Some(ModelLimits {
                context_window: 100_000,
                max_output_tokens: 4096,
            }))
        }
        fn estimate_request(&self, _: &ModelRequest) -> Result<Option<TokenEstimate>, String> {
            Ok(Some(TokenEstimate {
                tokens: 10_000,
                estimated: true,
            }))
        }
        fn summarize(&self, _: &SummaryRequest) -> Result<SummaryResponse, String> {
            Ok(SummaryResponse {
                content: "User goal, decisions, and pending work".into(),
                usage: None,
                finish_reason: "stop".into(),
            })
        }
    }
    let definition = LocalAgentDefinition::from_toml(
        r#"
name="fake"
display_name="Fake"
model_provider="fake"
model="fake"
system_prompt="assistant"
conversation_summary_prompt="summarize"
conversation_title_prompt="title"
[memory]
enabled=true
"#,
    )
    .unwrap();
    let runtime = Runtime::new(AgentRegistry::new(vec![Box::new(LocalAgent::new(
        definition,
        Box::new(Model),
    ))]));
    let data = tempfile::tempdir().unwrap();
    let store = ConversationStore::at(data.path().into());
    let meta = store.create_turn("first", "answer").unwrap();
    for _ in 0..9 {
        store.append_turn(&meta.id, "more", "answer").unwrap();
    }
    let before = store.load(&meta.id).unwrap();
    let service = ConversationService::new(&runtime, &store, 40);
    let registry = TaskRegistry::builtins();
    let request = TaskRequest {
        task_id: "conversation.compact".into(),
        arguments: json!({}),
        conversation_id: Some(meta.id.clone()),
    };
    let lease = store.acquire_execution(&meta.id).unwrap();
    assert!(registry
        .run(request.clone(), &service, None, &mut |_| {})
        .unwrap_err()
        .contains("already"));
    drop(lease);
    let mut progress = vec![];
    let result = registry
        .run(request, &service, None, &mut |p| progress.push(p))
        .unwrap();
    assert_eq!(result.task_id, "conversation.compact");
    let TaskOutcome::Compaction(result) = result.outcome;
    assert_eq!(result.memory.unwrap().compacted_message_count, 10);
    assert!(!progress.is_empty());
    let after = store.load(&meta.id).unwrap();
    assert_eq!(after.messages, before.messages);
    assert_eq!(after.metadata, before.metadata);
    assert_eq!(after.activities, before.activities);
    assert_eq!(store.list().unwrap().len(), 1);
}
