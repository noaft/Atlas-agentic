use std::sync::{Arc, Mutex};

use atlas_agentic::{
    media::ImageDetail,
    permissions::ToolRisk,
    tools::{
        ActiveWindowState, CaptureScreenTool, CapturedImage, ComputerController, ComputerKey,
        ComputerUiState, CoordinateMode, GetUiStateTool, KeyDirection, KeyPressTool, MouseButton,
        MouseClickTool, MouseMoveTool, MouseScrollTool, Observation, Point, ScrollAxis,
        SharedComputerController, Size, Tool, ToolAttachment, ToolRegistry, TypeTextTool,
    },
};

#[derive(Debug, Clone, PartialEq)]
enum Event {
    Move(i32, i32, CoordinateMode),
    Click(MouseButton),
    Scroll(i32, ScrollAxis),
    Text(String),
    Key(ComputerKey, KeyDirection),
    Observe,
    Capture,
}

struct RecordingController {
    events: Arc<Mutex<Vec<Event>>>,
    state: ComputerUiState,
    fail_key: Option<(ComputerKey, KeyDirection)>,
    captured: CapturedImage,
}

impl ComputerController for RecordingController {
    fn move_mouse(&mut self, x: i32, y: i32, coordinate: CoordinateMode) -> Result<(), String> {
        self.events
            .lock()
            .unwrap()
            .push(Event::Move(x, y, coordinate));
        Ok(())
    }

    fn click(&mut self, button: MouseButton) -> Result<(), String> {
        self.events.lock().unwrap().push(Event::Click(button));
        Ok(())
    }

    fn scroll(&mut self, amount: i32, axis: ScrollAxis) -> Result<(), String> {
        self.events
            .lock()
            .unwrap()
            .push(Event::Scroll(amount, axis));
        Ok(())
    }

    fn type_text(&mut self, text: &str) -> Result<(), String> {
        self.events
            .lock()
            .unwrap()
            .push(Event::Text(text.to_string()));
        Ok(())
    }

    fn key_event(&mut self, key: ComputerKey, direction: KeyDirection) -> Result<(), String> {
        self.events.lock().unwrap().push(Event::Key(key, direction));
        if self.fail_key == Some((key, direction)) {
            Err("simulated key failure".to_string())
        } else {
            Ok(())
        }
    }

    fn ui_state(&mut self) -> Result<ComputerUiState, String> {
        self.events.lock().unwrap().push(Event::Observe);
        Ok(self.state.clone())
    }

    fn capture_primary_display(&mut self) -> Result<CapturedImage, String> {
        self.events.lock().unwrap().push(Event::Capture);
        Ok(self.captured.clone())
    }
}

fn controller(
    fail_key: Option<(ComputerKey, KeyDirection)>,
) -> (SharedComputerController, Arc<Mutex<Vec<Event>>>) {
    controller_with_capture(
        fail_key,
        CapturedImage {
            png: vec![137, 80, 78, 71],
            width: 2,
            height: 1,
        },
    )
}

fn controller_with_capture(
    fail_key: Option<(ComputerKey, KeyDirection)>,
    captured: CapturedImage,
) -> (SharedComputerController, Arc<Mutex<Vec<Event>>>) {
    let events = Arc::new(Mutex::new(Vec::new()));
    let state = ComputerUiState {
        pointer: Observation::Available {
            value: Point { x: 20, y: 30 },
        },
        primary_display: Observation::Available {
            value: Size {
                width: 1920,
                height: 1080,
            },
        },
        active_window: Observation::<ActiveWindowState>::Unavailable {
            reason: "window metadata unavailable".to_string(),
        },
    };
    let shared: SharedComputerController = Arc::new(Mutex::new(Box::new(RecordingController {
        events: Arc::clone(&events),
        state,
        fail_key,
        captured,
    })));
    (shared, events)
}

#[test]
fn definitions_and_risks_describe_the_seven_computer_tools() {
    let (controller, _) = controller(None);
    let tools: Vec<Box<dyn Tool>> = vec![
        Box::new(MouseMoveTool::new(Arc::clone(&controller))),
        Box::new(MouseClickTool::new(Arc::clone(&controller))),
        Box::new(MouseScrollTool::new(Arc::clone(&controller))),
        Box::new(TypeTextTool::new(Arc::clone(&controller))),
        Box::new(KeyPressTool::new(Arc::clone(&controller))),
        Box::new(GetUiStateTool::new(Arc::clone(&controller))),
        Box::new(CaptureScreenTool::new(controller)),
    ];

    let names = tools
        .iter()
        .map(|tool| tool.definition().name)
        .collect::<Vec<_>>();
    assert_eq!(
        names,
        [
            "mouse_move",
            "mouse_click",
            "mouse_scroll",
            "type_text",
            "key_press",
            "get_ui_state",
            "capture_screen"
        ]
    );
    assert_eq!(
        tools[0].risk(&serde_json::json!({"x": 1, "y": 2})).unwrap(),
        ToolRisk::ComputerControl
    );
    assert_eq!(
        tools[1].risk(&serde_json::json!({})).unwrap(),
        ToolRisk::ComputerControl
    );
    assert_eq!(
        tools[2].risk(&serde_json::json!({"delta_y": 1})).unwrap(),
        ToolRisk::ComputerControl
    );
    assert_eq!(
        tools[3]
            .risk(&serde_json::json!({"text": "hello"}))
            .unwrap(),
        ToolRisk::ComputerControl
    );
    assert_eq!(
        tools[4].risk(&serde_json::json!({"key": "enter"})).unwrap(),
        ToolRisk::ComputerControl
    );
    assert_eq!(
        tools[5].risk(&serde_json::json!({})).unwrap(),
        ToolRisk::ComputerObserve
    );
    assert_eq!(
        tools[6].risk(&serde_json::json!({})).unwrap(),
        ToolRisk::ComputerObserve
    );

    let click_schema = tools[1].definition().input_schema;
    assert_eq!(click_schema["properties"]["count"]["minimum"], 1);
    assert_eq!(click_schema["properties"]["count"]["maximum"], 3);
    let key_schema = tools[4].definition().input_schema;
    assert_eq!(key_schema["required"], serde_json::json!(["key"]));
}

#[test]
fn capture_screen_returns_an_original_png_attachment_and_validates_before_capture() {
    let (controller, events) = controller(None);
    let tool = CaptureScreenTool::new(controller);

    assert_eq!(
        tool.summarize(&serde_json::json!({})),
        r#"{"scope":"primary_display"}"#
    );
    assert!(events.lock().unwrap().is_empty());
    let result = tool.execute(&serde_json::json!({})).unwrap();
    assert!(result.message.contains("2x1"));
    assert!(result.message.contains("PNG"));
    assert_eq!(
        result.attachments,
        vec![ToolAttachment::Image(atlas_agentic::media::ImageData {
            mime_type: "image/png".to_string(),
            bytes: vec![137, 80, 78, 71],
            width: 2,
            height: 1,
            detail: ImageDetail::Original,
        })]
    );
    assert_eq!(*events.lock().unwrap(), [Event::Capture]);

    events.lock().unwrap().clear();
    assert!(tool
        .execute(&serde_json::json!({"unexpected": true}))
        .is_err());
    assert!(events.lock().unwrap().is_empty());
}

#[test]
fn capture_screen_rejects_invalid_controller_output() {
    for captured in [
        CapturedImage {
            png: vec![],
            width: 2,
            height: 1,
        },
        CapturedImage {
            png: vec![1],
            width: 0,
            height: 1,
        },
    ] {
        let (controller, _) = controller_with_capture(None, captured);
        assert!(CaptureScreenTool::new(controller)
            .execute(&serde_json::json!({}))
            .is_err());
    }
}

#[test]
fn mouse_tools_apply_defaults_and_preserve_event_order() {
    let (controller, events) = controller(None);
    MouseMoveTool::new(Arc::clone(&controller))
        .execute(&serde_json::json!({"x": 50, "y": 60}))
        .unwrap();
    MouseMoveTool::new(Arc::clone(&controller))
        .execute(&serde_json::json!({"x": -5, "y": 4, "coordinate": "relative"}))
        .unwrap();
    MouseClickTool::new(Arc::clone(&controller))
        .execute(&serde_json::json!({"button": "right", "count": 2}))
        .unwrap();
    MouseScrollTool::new(controller)
        .execute(&serde_json::json!({"delta_x": -2, "delta_y": 3}))
        .unwrap();

    assert_eq!(
        *events.lock().unwrap(),
        [
            Event::Move(50, 60, CoordinateMode::Absolute),
            Event::Move(-5, 4, CoordinateMode::Relative),
            Event::Click(MouseButton::Right),
            Event::Click(MouseButton::Right),
            Event::Scroll(-2, ScrollAxis::Horizontal),
            Event::Scroll(3, ScrollAxis::Vertical),
        ]
    );
}

#[test]
fn mouse_tools_reject_invalid_inputs_before_sending_events() {
    let (controller, events) = controller(None);
    let move_tool = MouseMoveTool::new(Arc::clone(&controller));
    let click_tool = MouseClickTool::new(Arc::clone(&controller));
    let scroll_tool = MouseScrollTool::new(controller);

    assert!(move_tool
        .execute(&serde_json::json!({"x": 1, "y": 2, "coordinate": "screen"}))
        .is_err());
    for args in [
        serde_json::json!({"button": "primary"}),
        serde_json::json!({"count": 0}),
        serde_json::json!({"count": 4}),
    ] {
        assert!(click_tool.execute(&args).is_err());
    }
    for args in [
        serde_json::json!({}),
        serde_json::json!({"delta_x": 0, "delta_y": 0}),
        serde_json::json!({"delta_x": 101}),
        serde_json::json!({"delta_y": -101}),
    ] {
        assert!(scroll_tool.execute(&args).is_err());
    }
    assert!(events.lock().unwrap().is_empty());
}

#[test]
fn type_text_validates_unicode_scalars_and_keeps_approval_summary_bounded() {
    let (controller, events) = controller(None);
    let tool = TypeTextTool::new(controller);

    let result = tool
        .execute(&serde_json::json!({"text": "Xin chào 👋"}))
        .unwrap();
    assert!(result.message.contains("10 characters"));
    assert_eq!(
        *events.lock().unwrap(),
        [Event::Text("Xin chào 👋".to_string())]
    );

    let summary = tool.summarize(&serde_json::json!({"text": "a".repeat(500)}));
    assert!(summary.contains("500"));
    assert!(summary.chars().count() < 240);
    assert!(!summary.contains(&"a".repeat(200)));

    for text in [String::new(), "has\0null".to_string(), "界".repeat(10_001)] {
        assert!(tool.execute(&serde_json::json!({"text": text})).is_err());
    }
    assert_eq!(events.lock().unwrap().len(), 1);
}

#[test]
fn key_press_supports_named_character_function_keys_modifiers_and_repeats() {
    let (controller, events) = controller(None);
    let tool = KeyPressTool::new(controller);

    tool.execute(&serde_json::json!({
        "key": "x",
        "modifiers": ["control", "shift"],
        "repeat": 2
    }))
    .unwrap();
    tool.execute(&serde_json::json!({"key": "F12"})).unwrap();
    tool.execute(&serde_json::json!({"key": "page_down"}))
        .unwrap();

    assert_eq!(
        *events.lock().unwrap(),
        [
            Event::Key(ComputerKey::Control, KeyDirection::Press),
            Event::Key(ComputerKey::Shift, KeyDirection::Press),
            Event::Key(ComputerKey::Unicode('x'), KeyDirection::Click),
            Event::Key(ComputerKey::Unicode('x'), KeyDirection::Click),
            Event::Key(ComputerKey::Shift, KeyDirection::Release),
            Event::Key(ComputerKey::Control, KeyDirection::Release),
            Event::Key(ComputerKey::Function(12), KeyDirection::Click),
            Event::Key(ComputerKey::PageDown, KeyDirection::Click),
        ]
    );
}

#[test]
fn key_press_authorization_summary_shows_the_requested_shortcut() {
    let (controller, _) = controller(None);
    let registry = ToolRegistry::new(vec![Box::new(KeyPressTool::new(controller))]);
    let (_, summary) = registry
        .authorization(
            "key_press",
            &serde_json::json!({
                "key": "Delete",
                "modifiers": ["control", "alt"],
                "repeat": 2
            }),
        )
        .unwrap();

    assert!(summary.contains("Delete"));
    assert!(summary.contains("control"));
    assert!(summary.contains("alt"));
    assert!(summary.contains("2"));
    assert!(!summary.contains("[redacted]"));
}

#[test]
fn key_press_releases_modifiers_after_failure_and_rejects_bad_input_early() {
    let failing = Some((ComputerKey::Unicode('x'), KeyDirection::Click));
    let (controller, events) = controller(failing);
    let tool = KeyPressTool::new(controller);
    let error = tool
        .execute(&serde_json::json!({
            "key": "x",
            "modifiers": ["alt", "meta"]
        }))
        .err()
        .unwrap();
    assert!(error.contains("simulated key failure"));
    assert_eq!(
        *events.lock().unwrap(),
        [
            Event::Key(ComputerKey::Alt, KeyDirection::Press),
            Event::Key(ComputerKey::Meta, KeyDirection::Press),
            Event::Key(ComputerKey::Unicode('x'), KeyDirection::Click),
            Event::Key(ComputerKey::Meta, KeyDirection::Release),
            Event::Key(ComputerKey::Alt, KeyDirection::Release),
        ]
    );

    events.lock().unwrap().clear();
    for args in [
        serde_json::json!({"key": ""}),
        serde_json::json!({"key": "unknown_key"}),
        serde_json::json!({"key": "x", "modifiers": ["control", "control"]}),
        serde_json::json!({"key": "x", "modifiers": ["hyper"]}),
        serde_json::json!({"key": "x", "repeat": 0}),
        serde_json::json!({"key": "x", "repeat": 11}),
        serde_json::json!({"key": "\u{0000}", "modifiers": ["control"]}),
    ] {
        assert!(tool.risk(&args).is_err(), "risk should fail: {args}");
        assert!(tool.execute(&args).is_err(), "args should fail: {args}");
    }
    assert!(events.lock().unwrap().is_empty());
}

#[test]
fn key_press_releases_a_modifier_even_when_its_press_reports_failure() {
    let failing = Some((ComputerKey::Shift, KeyDirection::Press));
    let (controller, events) = controller(failing);
    let error = KeyPressTool::new(controller)
        .execute(&serde_json::json!({
            "key": "x",
            "modifiers": ["control", "shift", "alt"]
        }))
        .err()
        .expect("modifier press should fail");

    assert!(error.contains("simulated key failure"));
    assert_eq!(
        *events.lock().unwrap(),
        [
            Event::Key(ComputerKey::Control, KeyDirection::Press),
            Event::Key(ComputerKey::Shift, KeyDirection::Press),
            Event::Key(ComputerKey::Shift, KeyDirection::Release),
            Event::Key(ComputerKey::Control, KeyDirection::Release),
        ]
    );
}

#[test]
fn get_ui_state_returns_useful_partial_structured_json() {
    let (controller, events) = controller(None);
    let result = GetUiStateTool::new(controller)
        .execute(&serde_json::json!({}))
        .unwrap();
    let state: serde_json::Value = serde_json::from_str(&result.message).unwrap();

    assert_eq!(state["pointer"]["status"], "available");
    assert_eq!(state["pointer"]["value"]["x"], 20);
    assert_eq!(state["primaryDisplay"]["value"]["width"], 1920);
    assert_eq!(state["activeWindow"]["status"], "unavailable");
    assert_eq!(
        state["activeWindow"]["reason"],
        "window metadata unavailable"
    );
    assert_eq!(*events.lock().unwrap(), [Event::Observe]);
}
