use atlas_agentic::conversations::{choose_block_count, retained_start, CompactionTrigger};

#[test]
fn manual_buckets_are_token_percentages_and_auto_uses_three() {
    for (used, expected) in [
        (29.99, 1),
        (30.0, 2),
        (59.99, 2),
        (60.0, 3),
        (89.99, 3),
        (90.0, 3),
    ] {
        assert_eq!(
            choose_block_count(CompactionTrigger::Manual, used),
            expected
        );
        assert_eq!(choose_block_count(CompactionTrigger::Automatic, used), 3);
    }
}
#[test]
fn boundary_preserves_ten_messages_and_whole_exchanges() {
    use atlas_agentic::conversations::{ConversationMessage, MessageRole};
    let messages: Vec<_> = (0..100)
        .map(|i| ConversationMessage {
            role: if i % 2 == 0 {
                MessageRole::User
            } else {
                MessageRole::Assistant
            },
            content: i.to_string(),
            created_at: chrono::Utc::now(),
        })
        .collect();
    assert_eq!(retained_start(&messages, 10), 90);
    assert_eq!(retained_start(&messages[..9], 10), 0);
    assert_eq!(retained_start(&messages, 11), 88);
}
