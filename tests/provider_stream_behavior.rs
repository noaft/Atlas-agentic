use atlas_agentic::models::sse::read_sse;
use std::io::Cursor;
#[test]
fn frames_multiline_data_comments_and_named_events() {
    let mut events = Vec::new();
    read_sse(
        &mut Cursor::new(
            ": ping\r\nevent: update\r\ndata: {\r\ndata: \"text\":\"hello 🌍\"}\r\n\r\n",
        ),
        &mut |e| {
            events.push(e);
            Ok(true)
        },
    )
    .unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].event.as_deref(), Some("update"));
    assert_eq!(events[0].data, "{\n\"text\":\"hello 🌍\"}");
}
#[test]
fn eof_event_is_delivered_but_adapter_must_verify_completion() {
    let mut events = Vec::new();
    read_sse(&mut Cursor::new("data: [DONE]"), &mut |e| {
        events.push(e.data);
        Ok(true)
    })
    .unwrap();
    assert_eq!(events, vec!["[DONE]"]);
}
