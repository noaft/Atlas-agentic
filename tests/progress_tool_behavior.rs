use atlas_agentic::{
    permissions::ToolRisk,
    tools::{Tool, ToolEffect, UpdateProgressTool},
};
use serde_json::json;

#[test]
fn progress_tool_returns_a_typed_effect_without_finishing_the_task() {
    let tool = UpdateProgressTool::new();
    let args = json!({"message": "I’ll inspect the files, then run the tests."});
    assert_eq!(tool.name(), "update_progress");
    assert_eq!(tool.risk(&args).unwrap(), ToolRisk::ReadOnly);
    let result = tool.execute(&args).unwrap();
    assert_eq!(
        result.effects,
        vec![ToolEffect::Progress(
            "I’ll inspect the files, then run the tests.".into()
        )]
    );
    assert!(result.attachments.is_empty());
    assert!(!result.message.contains("I’ll inspect"));
}

#[test]
fn progress_tool_requires_a_short_nonempty_message_and_rejects_other_fields() {
    let tool = UpdateProgressTool::new();
    for args in [
        json!({}),
        json!({"message":"  "}),
        json!({"message":null}),
        json!({"message":"x".repeat(1001)}),
        json!({"message":"Next step", "extra":true}),
    ] {
        assert!(tool.execute(&args).is_err(), "{args}");
    }
    let result = tool
        .execute(&json!({"message":"  Kiểm tra mã nguồn  "}))
        .unwrap();
    assert_eq!(
        result.effects,
        vec![ToolEffect::Progress("Kiểm tra mã nguồn".into())]
    );
}
