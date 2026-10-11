use std::{cell::RefCell, rc::Rc};

use atlas_agentic::{
    agents::{Agent, LocalAgent, LocalAgentDefinition},
    conversations::{ConversationMessage, MessageRole},
    media::{ImageData, ImageDetail},
    models::{provider_from_definition, ModelProvider, ModelRequest},
    tools::ToolDefinition,
};
use chrono::{TimeZone, Utc};

#[derive(Debug, PartialEq, Eq)]
struct CapturedRequest {
    history: Vec<ConversationMessage>,
    user_prompt: String,
    images: Vec<ImageData>,
    activity_context: Option<String>,
}

struct CapturingModel {
    captured: Rc<RefCell<Option<CapturedRequest>>>,
}

impl ModelProvider for CapturingModel {
    fn validate_images(&self, _images: &[ImageData]) -> Result<(), String> {
        Ok(())
    }

    fn complete(&self, request: &ModelRequest) -> Result<String, String> {
        *self.captured.borrow_mut() = Some(CapturedRequest {
            history: request.history.to_vec(),
            user_prompt: request.user_prompt.to_string(),
            images: request.images.to_vec(),
            activity_context: request.activity_context.map(str::to_string),
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

fn discard_delta(_: &str) {}

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
            images: vec![],
            activity_context: None,
        })
    );
}

#[test]
fn local_agent_forwards_activity_context_separately_from_messages() {
    let captured = Rc::new(RefCell::new(None));
    let agent = LocalAgent::new(
        LocalAgentDefinition::from_toml(
            r#"
name = "atlas"
display_name = "Atlas"
model_provider = "fake"
model = "fake"
system_prompt = "You are Atlas."
conversation_title_prompt = "Write a title."
"#,
        )
        .unwrap(),
        Box::new(CapturingModel {
            captured: Rc::clone(&captured),
        }),
    );
    let mut sink = discard_delta;

    agent
        .process_stream_with_context_and_images(
            "Newest question",
            &[],
            &[],
            Some("Recent Atlas activity:\n- searched files"),
            &[],
            &mut sink,
        )
        .unwrap();

    assert_eq!(
        captured
            .borrow()
            .as_ref()
            .unwrap()
            .activity_context
            .as_deref(),
        Some("Recent Atlas activity:\n- searched files")
    );
    assert!(captured.borrow().as_ref().unwrap().history.is_empty());
}

struct TextOnlyAgent;

impl Agent for TextOnlyAgent {
    fn name(&self) -> &str {
        "text-only"
    }
    fn system_prompt(&self) -> &str {
        "text only"
    }
    fn process(
        &self,
        _input: &str,
        _history: &[ConversationMessage],
        _tools: &[ToolDefinition],
    ) -> Result<String, String> {
        Ok("answer".to_string())
    }
    fn process_stream(
        &self,
        input: &str,
        history: &[ConversationMessage],
        tools: &[ToolDefinition],
        _on_delta: &mut dyn FnMut(&str),
    ) -> Result<String, String> {
        self.process(input, history, tools)
    }
    fn generate_conversation_title(&self, _first_input: &str) -> Result<String, String> {
        Ok("title".to_string())
    }
}

#[test]
fn default_agent_rejects_nonempty_images_but_local_agent_forwards_them() {
    let image = ImageData {
        mime_type: "image/png".to_string(),
        bytes: vec![1, 2, 3],
        width: 2,
        height: 1,
        detail: ImageDetail::Original,
    };
    let mut sink = discard_delta;
    let error = TextOnlyAgent
        .process_stream_with_images("inspect", &[], &[], std::slice::from_ref(&image), &mut sink)
        .unwrap_err();
    assert_eq!(error, "model provider does not support image input");

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

    assert_eq!(
        agent
            .process_stream_with_images(
                "inspect",
                &[],
                &[],
                std::slice::from_ref(&image),
                &mut sink,
            )
            .unwrap(),
        "answer"
    );
    assert_eq!(captured.borrow().as_ref().unwrap().images, vec![image]);
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
