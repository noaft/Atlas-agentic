use atlas_agentic::conversations::{
    render_activity_context, sanitize_arguments, sanitize_result, ActivityCollector, ActivityKind,
    ActivityOutcome, ActivityRecord, ActivityTurn,
};
use chrono::{TimeZone, Utc};
use serde_json::json;

fn record(sequence: u32, summary: impl Into<String>) -> ActivityRecord {
    ActivityRecord {
        sequence,
        kind: ActivityKind::Status,
        tool: None,
        arguments: None,
        outcome: None,
        summary: summary.into(),
        details: None,
        created_at: Utc.timestamp_opt(1_760_000_000, 0).unwrap(),
    }
}

#[test]
fn activity_turn_round_trips_as_one_versioned_json_line() {
    let turn = ActivityTurn {
        version: 1,
        turn_id: "turn-1".to_string(),
        assistant_message_index: 3,
        duration_ms: Some(72_000),
        records: vec![record(0, "Thinking")],
    };

    let line = serde_json::to_string(&turn).unwrap();
    assert!(!line.contains('\n'));
    assert_eq!(serde_json::from_str::<ActivityTurn>(&line).unwrap(), turn);
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&line).unwrap()["version"],
        1
    );
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&line).unwrap()["durationMs"],
        72_000
    );
}

#[test]
fn legacy_activity_turn_without_duration_loads_normally() {
    let line = r#"{"version":1,"turnId":"legacy","assistantMessageIndex":1,"records":[]}"#;

    let turn = serde_json::from_str::<ActivityTurn>(line).unwrap();

    assert_eq!(turn.duration_ms, None);
}

#[test]
fn sanitizer_redacts_nested_mixed_case_secret_keys() {
    let rendered = sanitize_arguments(&json!({
        "items": [{"Authorization": "Bearer private"}],
        "apiKey": "sk-private",
        "ordinary": "visible"
    }));

    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&rendered).unwrap(),
        json!({
            "apiKey": "[redacted]",
            "items": [{"Authorization": "[redacted]"}],
            "ordinary": "visible"
        })
    );
}

#[test]
fn sanitizer_truncates_unicode_at_argument_and_detail_limits() {
    let arguments = sanitize_arguments(&json!({"text": "é".repeat(3_000)}));
    let details = sanitize_result(&"界".repeat(10_000));

    assert_eq!(arguments.chars().count(), 2_048);
    assert!(arguments.ends_with("… [truncated]"));
    assert!(serde_json::from_str::<serde_json::Value>(&arguments).is_err());
    assert_eq!(details.chars().count(), 8_192);
    assert!(details.ends_with("… [truncated]"));
}

#[test]
fn collector_enforces_the_per_turn_detail_budget() {
    let mut collector = ActivityCollector::new();
    collector.status("Thinking");
    collector.tool_requested("search_files", &json!({"query": "needle"}));
    collector.tool_finished(
        "search_files",
        true,
        "Search completed",
        &"a".repeat(20_000),
    );
    collector.tool_finished("read_file", true, "Read completed", &"b".repeat(20_000));
    collector.tool_finished("pwd", true, "Path read", &"c".repeat(20_000));
    collector.tool_finished("ls", true, "Directory listed", &"d".repeat(20_000));
    collector.tool_finished("current_time", true, "Time read", &"e".repeat(20_000));
    collector.tool_denied("delete_file");
    let turn = collector.into_turn("turn-budget", 1);

    assert_eq!(
        turn.records
            .iter()
            .filter_map(|entry| entry.details.as_ref())
            .map(|value| value.chars().count())
            .sum::<usize>(),
        32_768
    );
    assert_eq!(
        turn.records
            .iter()
            .map(|entry| entry.sequence)
            .collect::<Vec<_>>(),
        vec![0, 1, 2, 3, 4, 5, 6, 7]
    );
    assert_eq!(turn.records[7].outcome, Some(ActivityOutcome::Denied));
}

#[test]
fn context_keeps_newest_complete_summaries_within_8192_characters() {
    let oldest = ActivityTurn {
        version: 1,
        turn_id: "oldest".to_string(),
        assistant_message_index: 1,
        duration_ms: None,
        records: vec![record(0, "oldest should stay behind the oversized gap")],
    };
    let old = ActivityTurn {
        version: 1,
        turn_id: "old".to_string(),
        assistant_message_index: 3,
        duration_ms: None,
        records: vec![record(0, format!("old {}", "x".repeat(8_000)))],
    };
    let newest_summary = format!("new {}", "y".repeat(4_000));
    let newest = ActivityTurn {
        version: 1,
        turn_id: "new".to_string(),
        assistant_message_index: 5,
        duration_ms: None,
        records: vec![record(0, newest_summary.clone())],
    };

    let context = render_activity_context(&[oldest, old, newest]);

    assert!(context.starts_with("Recent Atlas activity (facts from earlier successful turns):\n"));
    assert!(context.contains(&newest_summary));
    assert!(!context.contains("old "));
    assert!(!context.contains("oldest should"));
    assert!(context.chars().count() <= 8_192);
}

#[test]
fn capture_screen_record_drops_image_payload_details() {
    let mut collector = ActivityCollector::new();
    collector.tool_requested("capture_screen", &json!({}));
    collector.tool_finished(
        "capture_screen",
        true,
        "Captured primary display (1920x1080 PNG)",
        "data:image/png;base64,PRIVATEPIXELS",
    );
    let turn = collector.into_turn("capture", 1);

    assert_eq!(turn.records[1].details, None);
    assert!(turn.records[1].summary.contains("1920x1080"));
    assert!(!serde_json::to_string(&turn)
        .unwrap()
        .contains("PRIVATEPIXELS"));
}
