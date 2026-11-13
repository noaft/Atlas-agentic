use atlas_agentic::{conversations::*, turns::TurnInput};
use chrono::{Duration, Utc};

#[test]
fn multiple_user_inputs_remain_distinct_and_activity_tracks_assistant() {
    let dir = tempfile::tempdir().unwrap();
    let store = ConversationStore::at(dir.path().into());
    let input = TurnInput {
        id: "steer".into(),
        content: "also check this".into(),
        created_at: Utc::now() - Duration::seconds(2),
    };
    let activity = ActivityCollector::new().into_unassociated_turn();
    let metadata = store
        .create_turn_with_inputs_and_activity(
            "first",
            std::slice::from_ref(&input),
            "answer",
            None,
            None,
            activity,
        )
        .unwrap();
    let saved = store.load(&metadata.id).unwrap();
    assert_eq!(
        saved
            .messages
            .iter()
            .map(|m| m.content.as_str())
            .collect::<Vec<_>>(),
        ["first", "also check this", "answer"]
    );
    assert_eq!(saved.messages[1].created_at, input.created_at);
    assert!(saved.messages[0].created_at <= saved.messages[1].created_at);
    assert_eq!(saved.activities[0].assistant_message_index, 2);
    let metadata = store
        .append_turn_with_inputs_and_activity(
            &metadata.id,
            "next",
            &[input],
            "done",
            None,
            None,
            ActivityCollector::new().into_unassociated_turn(),
        )
        .unwrap();
    let saved = store.load(&metadata.id).unwrap();
    assert_eq!(metadata.message_count, 6);
    assert_eq!(saved.activities[1].assistant_message_index, 5);
}

#[test]
fn recent_history_keeps_all_users_in_the_same_exchange() {
    let now = Utc::now();
    let messages = [
        MessageRole::User,
        MessageRole::Assistant,
        MessageRole::User,
        MessageRole::User,
        MessageRole::Assistant,
    ]
    .into_iter()
    .map(|role| ConversationMessage {
        role,
        content: "message".into(),
        created_at: now,
    })
    .collect::<Vec<_>>();
    assert_eq!(retained_start(&messages, 2), 2);
    assert_eq!(retained_start(&messages, 1), 2);
    assert_eq!(retained_start(&messages, 0), messages.len());
}

#[test]
fn interrupted_history_labels_every_assistant_including_old_empty_responses() {
    let dir = tempfile::tempdir().unwrap();
    let store = ConversationStore::at(dir.path().into());
    let mut activity = ActivityCollector::new().into_unassociated_turn();
    activity.interrupted = true;
    let metadata = store
        .create_turn_with_activity("first", "", None, None, activity.clone())
        .unwrap();
    for _ in 0..40 {
        store
            .append_turn_with_activity(
                &metadata.id,
                "next",
                "partial",
                None,
                None,
                activity.clone(),
            )
            .unwrap();
    }
    let saved = store.load(&metadata.id).unwrap();
    let mut history = saved.messages.clone();
    annotate_interrupted_history(&mut history, 0, &saved.activities);
    assert!(saved.messages[1].content.is_empty());
    for message in history
        .iter()
        .filter(|message| message.role == MessageRole::Assistant)
    {
        assert!(message.content.contains("incomplete"));
    }
    let mut suffix = saved.messages[2..].to_vec();
    annotate_interrupted_history(&mut suffix, 2, &saved.activities);
    assert!(suffix[1].content.contains("incomplete"));
    assert_eq!(suffix[0].content, "next");
}

#[test]
fn legacy_history_window_keeps_original_with_selected_supplement() {
    let now = Utc::now();
    let messages = [
        (MessageRole::User, "old question"),
        (MessageRole::Assistant, "old answer"),
        (MessageRole::User, "original"),
        (MessageRole::User, "supplement"),
        (MessageRole::Assistant, "answer"),
    ]
    .into_iter()
    .map(|(role, content)| ConversationMessage {
        role,
        content: content.into(),
        created_at: now,
    })
    .collect::<Vec<_>>();
    assert_eq!(
        select_recent_history(&messages, 2)
            .iter()
            .map(|message| message.content.as_str())
            .collect::<Vec<_>>(),
        ["original", "supplement", "answer"]
    );
    assert!(select_recent_history(&messages, 1).is_empty());
}
