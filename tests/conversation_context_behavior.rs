use std::sync::{Mutex, OnceLock};

use atlas_agentic::conversations::{
    history_limit_from_env, select_recent_history, ConversationId, ConversationMessage,
    MessageRole, DEFAULT_HISTORY_MESSAGES,
};
use chrono::{TimeZone, Utc};

fn environment_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

fn message(role: MessageRole, content: impl Into<String>, second: i64) -> ConversationMessage {
    ConversationMessage {
        role,
        content: content.into(),
        created_at: Utc.timestamp_opt(second, 0).single().unwrap(),
    }
}

#[test]
fn generated_conversation_id_round_trips_through_uuid_parsing() {
    let id = ConversationId::new();

    let parsed = ConversationId::parse(&id.to_string()).unwrap();

    assert_eq!(parsed, id);
}

#[test]
fn conversation_id_rejects_path_like_values() {
    for value in [
        "../history",
        "conversations/record",
        "/tmp/record",
        "record.jsonl",
    ] {
        assert!(
            ConversationId::parse(value).is_err(),
            "{value} should be rejected"
        );
    }
}

#[test]
fn history_limit_defaults_to_forty_when_unset() {
    let _guard = environment_lock().lock().unwrap();
    unsafe { std::env::remove_var("ATLAS_HISTORY_MESSAGES") };

    assert_eq!(history_limit_from_env().unwrap(), DEFAULT_HISTORY_MESSAGES);
    assert_eq!(DEFAULT_HISTORY_MESSAGES, 40);
}

#[test]
fn history_limit_uses_positive_environment_override() {
    let _guard = environment_lock().lock().unwrap();
    unsafe { std::env::set_var("ATLAS_HISTORY_MESSAGES", "12") };

    assert_eq!(history_limit_from_env().unwrap(), 12);

    unsafe { std::env::remove_var("ATLAS_HISTORY_MESSAGES") };
}

#[test]
fn history_limit_rejects_zero_and_non_numeric_overrides() {
    let _guard = environment_lock().lock().unwrap();

    for value in ["0", "many"] {
        unsafe { std::env::set_var("ATLAS_HISTORY_MESSAGES", value) };
        assert!(
            history_limit_from_env().is_err(),
            "{value} should be rejected"
        );
    }

    unsafe { std::env::remove_var("ATLAS_HISTORY_MESSAGES") };
}

#[test]
fn recent_history_keeps_at_most_forty_newest_messages_in_order() {
    let messages = (0..46)
        .map(|index| {
            let role = if index % 2 == 0 {
                MessageRole::User
            } else {
                MessageRole::Assistant
            };
            message(role, format!("message-{index}"), index)
        })
        .collect::<Vec<_>>();

    let selected = select_recent_history(&messages, DEFAULT_HISTORY_MESSAGES);

    assert_eq!(selected.len(), 40);
    assert_eq!(selected.first().unwrap().content, "message-6");
    assert_eq!(selected.last().unwrap().content, "message-45");
    assert_eq!(
        selected
            .iter()
            .map(|message| message.content.clone())
            .collect::<Vec<_>>(),
        (6..46)
            .map(|index| format!("message-{index}"))
            .collect::<Vec<_>>()
    );
}

#[test]
fn recent_history_drops_leading_assistant_when_limit_cuts_a_turn() {
    let messages = vec![
        message(MessageRole::User, "one", 1),
        message(MessageRole::Assistant, "one answer", 2),
        message(MessageRole::User, "two", 3),
        message(MessageRole::Assistant, "two answer", 4),
    ];

    let selected = select_recent_history(&messages, 3);

    assert_eq!(
        selected,
        vec![
            message(MessageRole::User, "two", 3),
            message(MessageRole::Assistant, "two answer", 4),
        ]
    );
}
