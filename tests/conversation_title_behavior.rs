use std::{cell::RefCell, rc::Rc};

use atlas_agentic::{
    agents::{Agent, LocalAgent, LocalAgentDefinition},
    conversations::sanitize_title,
    models::{ModelProvider, ModelRequest},
};

#[derive(Debug, PartialEq, Eq)]
struct CapturedRequest {
    system_prompt: String,
    user_prompt: String,
    history_length: usize,
    tool_count: usize,
}

struct CapturingTitleModel {
    captured: Rc<RefCell<Option<CapturedRequest>>>,
}

impl ModelProvider for CapturingTitleModel {
    fn complete(&self, request: &ModelRequest) -> Result<String, String> {
        *self.captured.borrow_mut() = Some(CapturedRequest {
            system_prompt: request.system_prompt.to_string(),
            user_prompt: request.user_prompt.to_string(),
            history_length: request.history.len(),
            tool_count: request.tools.len(),
        });
        Ok("Atlas conversation titles".to_string())
    }
}

struct FailingTitleModel;

impl ModelProvider for FailingTitleModel {
    fn complete(&self, _request: &ModelRequest) -> Result<String, String> {
        Err("title provider unavailable".to_string())
    }
}

fn definition() -> LocalAgentDefinition {
    LocalAgentDefinition::from_toml(
        r#"
name = "atlas"
display_name = "Atlas"
model_provider = "fake"
model = "fake"
temperature = 0.3
top_p = 0.9
system_prompt = "You are Atlas."
conversation_title_prompt = "Return a plain-text conversation title."
"#,
    )
    .unwrap()
}

#[test]
fn title_generation_uses_only_the_title_prompt_and_first_input() {
    let captured = Rc::new(RefCell::new(None));
    let agent = LocalAgent::new(
        definition(),
        Box::new(CapturingTitleModel {
            captured: Rc::clone(&captured),
        }),
    );

    let title = agent
        .generate_conversation_title("Help me understand Atlas")
        .unwrap();

    assert_eq!(title, "Atlas conversation titles");
    assert_eq!(
        *captured.borrow(),
        Some(CapturedRequest {
            system_prompt: "Return a plain-text conversation title.".to_string(),
            user_prompt: "Help me understand Atlas".to_string(),
            history_length: 0,
            tool_count: 0,
        })
    );
}

#[test]
fn title_generation_propagates_provider_errors() {
    let agent = LocalAgent::new(definition(), Box::new(FailingTitleModel));

    let error = agent
        .generate_conversation_title("Help me understand Atlas")
        .unwrap_err();

    assert_eq!(error, "title provider unavailable");
}

#[test]
fn sanitize_title_collapses_whitespace_and_removes_surrounding_quotes() {
    assert_eq!(
        sanitize_title("  \n  \"  Understand\tAtlas\r\nruntime  \"  "),
        Some("Understand Atlas runtime".to_string())
    );
    assert_eq!(
        sanitize_title("'Debug the conversation store'"),
        Some("Debug the conversation store".to_string())
    );
}

#[test]
fn sanitize_title_rejects_blank_or_quote_only_output() {
    assert_eq!(sanitize_title(" \n\t "), None);
    assert_eq!(sanitize_title("  \"'\"  "), None);
}

#[test]
fn sanitize_title_limits_unicode_output_to_sixty_scalar_characters() {
    let raw = format!("{}tail", "🦀".repeat(60));

    let title = sanitize_title(&raw).unwrap();

    assert_eq!(title, "🦀".repeat(60));
    assert_eq!(title.chars().count(), 60);
}
