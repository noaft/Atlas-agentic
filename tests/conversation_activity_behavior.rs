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
        usage: None,
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
        usage: None,
        records: vec![record(0, "oldest should stay behind the oversized gap")],
    };
    let old = ActivityTurn {
        version: 1,
        turn_id: "old".to_string(),
        assistant_message_index: 3,
        duration_ms: None,
        usage: None,
        records: vec![record(0, format!("old {}", "x".repeat(8_000)))],
    };
    let newest_summary = format!("new {}", "y".repeat(4_000));
    let newest = ActivityTurn {
        version: 1,
        turn_id: "new".to_string(),
        assistant_message_index: 5,
        duration_ms: None,
        usage: None,
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

#[test]
fn large_process_details_keep_position_metadata_in_valid_bounded_json() {
    let details = json!({
        "session_id":"owned", "running":true, "tail_lines":50,
        "stdout":format!("old\n{}\nlatest\n", "🌍\"".repeat(10_000)),
        "stderr":"warning\n", "stdout_truncated":false,
        "stdout_view":{"start_byte":100,"end_byte":50112,"start_line":8,"end_line":10,"older_output_dropped":true},
        "stderr_view":{"start_byte":0,"end_byte":8,"start_line":1,"end_line":1,"older_output_dropped":false}
    });
    let bounded = sanitize_result(&details.to_string());
    assert!(bounded.chars().count() <= 8192);
    let parsed: serde_json::Value = serde_json::from_str(&bounded).unwrap();
    assert_eq!(parsed["stdout_view"], details["stdout_view"]);
    assert_eq!(parsed["stderr_view"], details["stderr_view"]);
    assert_eq!(parsed["display_truncated"], true);
    assert!(parsed["stdout"].as_str().unwrap().ends_with("latest\n"));
    assert_eq!(parsed["stderr"], "warning\n");
    let mut collector = ActivityCollector::new();
    collector.tool_finished("read_process", true, "Read process", &details.to_string());
    let turn = collector.into_turn("tail", 1);
    let saved: serde_json::Value =
        serde_json::from_str(turn.records[0].details.as_ref().unwrap()).unwrap();
    assert_eq!(saved["stdout_view"], details["stdout_view"]);
}

#[test]
fn process_metadata_fallback_respects_even_a_one_character_remaining_turn_budget() {
    let mut collector = ActivityCollector::new();
    for size in [8192, 8192, 8192, 8191] {
        collector.tool_finished("exec_command", true, "Read output", &"a".repeat(size));
    }
    let details = json!({"session_id":"owned","stdout":"latest\n","stderr":"", "stdout_view":{},"stderr_view":{}});
    collector.tool_finished("read_process", true, "Read process", &details.to_string());
    let turn = collector.into_turn("budget", 1);
    let count: usize = turn
        .records
        .iter()
        .filter_map(|record| record.details.as_ref())
        .map(|text| text.chars().count())
        .sum();
    assert!(count <= 32768, "stored {count} characters");
}
