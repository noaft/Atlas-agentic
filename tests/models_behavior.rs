use std::{cell::RefCell, rc::Rc};

use atlas_agentic::{
    agents::{Agent, LocalAgent, LocalAgentDefinition},
    conversations::{ConversationMessage, MessageRole},
    models::{provider_from_definition, ModelProvider, ModelRequest},
};
use chrono::{TimeZone, Utc};

#[derive(Debug, PartialEq, Eq)]
struct CapturedRequest {
    history: Vec<ConversationMessage>,
    user_prompt: String,
}

struct CapturingModel {
    captured: Rc<RefCell<Option<CapturedRequest>>>,
}

impl ModelProvider for CapturingModel {
    fn complete(&self, request: &ModelRequest) -> Result<String, String> {
        *self.captured.borrow_mut() = Some(CapturedRequest {
            history: request.history.to_vec(),
            user_prompt: request.user_prompt.to_string(),
        });
        Ok("answer".to_string())
    }
}

fn message(role: MessageRole, content: &str, second: u32) -> ConversationMessage {
    ConversationMessage {
        role,
        content: content.to_string(),
        created_at: Utc.with_ymd_and_hms(2026, 10, 4, 12, 0, second).unwrap(),
    }
}

#[test]
fn local_agent_keeps_saved_history_separate_from_the_new_user_input() {
    let captured = Rc::new(RefCell::new(None));
    let agent = LocalAgent::new(
        LocalAgentDefinition::from_toml(
            r#"
name = "atlas"
display_name = "Atlas"
model_provider = "fake"
model = "fake"
system_prompt = "You are Atlas."
conversation_title_prompt = "Write a short conversation title."
"#,
        )
        .unwrap(),
        Box::new(CapturingModel {
            captured: Rc::clone(&captured),
        }),
    );
    let history = vec![
        message(MessageRole::User, "Earlier question", 0),
        message(MessageRole::Assistant, "Earlier answer", 1),
    ];

    let response = agent.process("Newest question", &history, &[]).unwrap();

    assert_eq!(response, "answer");
    assert_eq!(
        *captured.borrow(),
        Some(CapturedRequest {
            history,
            user_prompt: "Newest question".to_string(),
        })
    );
}

#[test]
fn provider_factory_rejects_unknown_provider() {
    let definition = LocalAgentDefinition::from_toml(
        r#"
name = "atlas"
display_name = "Atlas"
model_provider = "unknown"
model = "some-model"
system_prompt = "You are Atlas."
conversation_title_prompt = "Write a short conversation title."
"#,
    )
    .unwrap();

    let result = provider_from_definition(&definition);

    match result {
        Ok(_) => panic!("expected unknown provider error"),
        Err(error) => assert_eq!(error, "unknown model provider: unknown"),
    }
}

#[test]
fn provider_factory_uses_deepseek_provider_name() {
    let definition = LocalAgentDefinition::from_toml(
        r#"
name = "atlas"
display_name = "Atlas"
model_provider = "deepseek"
model = "deepseek-flash"
system_prompt = "You are Atlas."
conversation_title_prompt = "Write a short conversation title."
"#,
    )
    .unwrap();

    let provider = provider_from_definition(&definition);

    match provider {
        Ok(_) => panic!("expected missing DeepSeek key error"),
        Err(error) => assert_eq!(error, "DEEPSEEK_API_KEY is required"),
    }
}
