use std::collections::HashSet;

use serde::Deserialize;

use crate::permissions::ToolRisk;

use super::{
    ComputerKey, CoordinateMode, KeyDirection, MouseButton, ScrollAxis, SharedComputerController,
};
use crate::tools::{Tool, ToolDefinition, ToolResult};

const MAX_TEXT_CHARACTERS: usize = 10_000;
const TEXT_PREVIEW_CHARACTERS: usize = 80;

pub struct MouseMoveTool {
    controller: SharedComputerController,
}

impl MouseMoveTool {
    pub fn new(controller: SharedComputerController) -> Self {
        Self { controller }
    }
}

impl Tool for MouseMoveTool {
    fn name(&self) -> &str {
        "mouse_move"
    }

    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: self.name().to_string(),
            description:
                "Move the mouse on the active desktop using absolute or relative coordinates."
                    .to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "required": ["x", "y"],
                "properties": {
                    "x": {"type": "integer"},
                    "y": {"type": "integer"},
                    "coordinate": {
                        "type": "string",
                        "enum": ["absolute", "relative"],
                        "default": "absolute"
                    }
                }
            }),
        }
    }

    fn risk(&self, args: &serde_json::Value) -> Result<ToolRisk, String> {
        parse_mouse_move(args)?;
        Ok(ToolRisk::ComputerControl)
    }

    fn execute(&self, args: &serde_json::Value) -> Result<ToolResult, String> {
        let args = parse_mouse_move(args)?;
        controller(&self.controller)?.move_mouse(args.x, args.y, args.coordinate)?;
        Ok(ToolResult::text(format!(
            "Moved mouse to ({}, {}) using {} coordinates.",
            args.x,
            args.y,
            coordinate_name(args.coordinate)
        )))
    }
}

pub struct MouseClickTool {
    controller: SharedComputerController,
}

impl MouseClickTool {
    pub fn new(controller: SharedComputerController) -> Self {
        Self { controller }
    }
}

impl Tool for MouseClickTool {
    fn name(&self) -> &str {
        "mouse_click"
    }

    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: self.name().to_string(),
            description: "Click a mouse button in the application active at execution time."
                .to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "button": {
                        "type": "string",
                        "enum": ["left", "middle", "right", "back", "forward"],
                        "default": "left"
                    },
                    "count": {"type": "integer", "minimum": 1, "maximum": 3, "default": 1}
                }
            }),
        }
    }

    fn risk(&self, args: &serde_json::Value) -> Result<ToolRisk, String> {
        parse_mouse_click(args)?;
        Ok(ToolRisk::ComputerControl)
    }

    fn execute(&self, args: &serde_json::Value) -> Result<ToolResult, String> {
        let args = parse_mouse_click(args)?;
        let mut controller = controller(&self.controller)?;
        for _ in 0..args.count {
            controller.click(args.button)?;
        }
        Ok(ToolResult::text(format!(
            "Clicked {} mouse button {} time(s).",
            button_name(args.button),
            args.count
        )))
    }
}

pub struct MouseScrollTool {
    controller: SharedComputerController,
}

impl MouseScrollTool {
    pub fn new(controller: SharedComputerController) -> Self {
        Self { controller }
    }
}

impl Tool for MouseScrollTool {
    fn name(&self) -> &str {
        "mouse_scroll"
    }

    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: self.name().to_string(),
            description:
                "Scroll horizontally or vertically in the application active at execution time."
                    .to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "delta_x": {"type": "integer", "minimum": -100, "maximum": 100, "default": 0},
                    "delta_y": {"type": "integer", "minimum": -100, "maximum": 100, "default": 0}
                }
            }),
        }
    }

    fn risk(&self, args: &serde_json::Value) -> Result<ToolRisk, String> {
        parse_mouse_scroll(args)?;
        Ok(ToolRisk::ComputerControl)
    }

    fn execute(&self, args: &serde_json::Value) -> Result<ToolResult, String> {
        let args = parse_mouse_scroll(args)?;
        let mut controller = controller(&self.controller)?;
        if args.delta_x != 0 {
            controller.scroll(args.delta_x, ScrollAxis::Horizontal)?;
        }
        if args.delta_y != 0 {
            controller.scroll(args.delta_y, ScrollAxis::Vertical)?;
        }
        Ok(ToolResult::text(format!(
            "Scrolled mouse by ({}, {}).",
            args.delta_x, args.delta_y
        )))
    }
}

pub struct TypeTextTool {
    controller: SharedComputerController,
}

impl TypeTextTool {
    pub fn new(controller: SharedComputerController) -> Self {
        Self { controller }
    }
}

impl Tool for TypeTextTool {
    fn name(&self) -> &str {
        "type_text"
    }

    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: self.name().to_string(),
            description: "Type Unicode text into the application active at execution time."
                .to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "required": ["text"],
                "properties": {"text": {"type": "string", "minLength": 1, "maxLength": MAX_TEXT_CHARACTERS}}
            }),
        }
    }

    fn risk(&self, args: &serde_json::Value) -> Result<ToolRisk, String> {
        parse_type_text(args)?;
        Ok(ToolRisk::ComputerControl)
    }

    fn summarize(&self, args: &serde_json::Value) -> String {
        match parse_type_text(args) {
            Ok(args) => {
                let count = args.text.chars().count();
                let mut preview = args
                    .text
                    .chars()
                    .take(TEXT_PREVIEW_CHARACTERS)
                    .collect::<String>();
                if count > TEXT_PREVIEW_CHARACTERS {
                    preview.push('…');
                }
                serde_json::json!({"characters": count, "preview": preview}).to_string()
            }
            Err(_) => crate::tools::summarize_arguments(args),
        }
    }

    fn execute(&self, args: &serde_json::Value) -> Result<ToolResult, String> {
        let args = parse_type_text(args)?;
        let count = args.text.chars().count();
        controller(&self.controller)?.type_text(&args.text)?;
        Ok(ToolResult::text(format!(
            "Typed {count} characters into the active application."
        )))
    }
}

pub struct KeyPressTool {
    controller: SharedComputerController,
}

impl KeyPressTool {
    pub fn new(controller: SharedComputerController) -> Self {
        Self { controller }
    }
}

impl Tool for KeyPressTool {
    fn name(&self) -> &str {
        "key_press"
    }

    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: self.name().to_string(),
            description:
                "Press a key with optional modifiers in the application active at execution time."
                    .to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "required": ["key"],
                "properties": {
                    "key": {"type": "string"},
                    "modifiers": {
                        "type": "array",
                        "items": {"type": "string", "enum": ["control", "alt", "shift", "meta"]},
                        "uniqueItems": true,
                        "default": []
                    },
                    "repeat": {"type": "integer", "minimum": 1, "maximum": 10, "default": 1}
                }
            }),
        }
    }

    fn risk(&self, args: &serde_json::Value) -> Result<ToolRisk, String> {
        parse_key_press(args)?;
        Ok(ToolRisk::ComputerControl)
    }

    fn summarize(&self, args: &serde_json::Value) -> String {
        match parse_key_press(args) {
            Ok(args) => serde_json::json!({
                "key": args.label,
                "modifiers": args.modifiers.iter().map(|key| modifier_name(*key)).collect::<Vec<_>>(),
                "repeat": args.repeat,
            })
            .to_string(),
            Err(_) => crate::tools::summarize_arguments(args),
        }
    }

    fn execute(&self, args: &serde_json::Value) -> Result<ToolResult, String> {
        let args = parse_key_press(args)?;
        let mut controller = controller(&self.controller)?;
        let mut pressed = Vec::new();
        let mut primary_error = None;

        for modifier in &args.modifiers {
            pressed.push(*modifier);
            match controller.key_event(*modifier, KeyDirection::Press) {
                Ok(()) => {}
                Err(error) => {
                    primary_error = Some(error);
                    break;
                }
            }
        }
        if primary_error.is_none() {
            for _ in 0..args.repeat {
                if let Err(error) = controller.key_event(args.key, KeyDirection::Click) {
                    primary_error = Some(error);
                    break;
                }
            }
        }

        let mut cleanup_error = None;
        for modifier in pressed.into_iter().rev() {
            if let Err(error) = controller.key_event(modifier, KeyDirection::Release) {
                cleanup_error.get_or_insert(error);
            }
        }
        if let Some(error) = primary_error {
            return Err(match cleanup_error {
                Some(cleanup) => format!("{error}; modifier cleanup also failed: {cleanup}"),
                None => error,
            });
        }
        if let Some(error) = cleanup_error {
            return Err(format!("failed to release modifier: {error}"));
        }

        Ok(ToolResult::text(format!(
            "Pressed {} {} time(s).",
            args.label, args.repeat
        )))
    }
}

fn controller(
    controller: &SharedComputerController,
) -> Result<std::sync::MutexGuard<'_, Box<dyn super::ComputerController>>, String> {
    controller
        .lock()
        .map_err(|_| "computer controller lock was poisoned".to_string())
}

#[derive(Deserialize)]
struct MouseMoveArgs {
    x: i32,
    y: i32,
    #[serde(default)]
    coordinate: CoordinateArg,
}

#[derive(Default, Deserialize)]
#[serde(rename_all = "lowercase")]
enum CoordinateArg {
    #[default]
    Absolute,
    Relative,
}

fn parse_mouse_move(args: &serde_json::Value) -> Result<ParsedMouseMove, String> {
    let args: MouseMoveArgs = serde_json::from_value(args.clone())
        .map_err(|error| format!("invalid mouse_move args: {error}"))?;
    Ok(ParsedMouseMove {
        x: args.x,
        y: args.y,
        coordinate: match args.coordinate {
            CoordinateArg::Absolute => CoordinateMode::Absolute,
            CoordinateArg::Relative => CoordinateMode::Relative,
        },
    })
}

struct ParsedMouseMove {
    x: i32,
    y: i32,
    coordinate: CoordinateMode,
}

#[derive(Deserialize)]
struct MouseClickArgs {
    #[serde(default)]
    button: ButtonArg,
    #[serde(default = "default_one")]
    count: u8,
}

#[derive(Default, Deserialize)]
#[serde(rename_all = "lowercase")]
enum ButtonArg {
    #[default]
    Left,
    Middle,
    Right,
    Back,
    Forward,
}

fn parse_mouse_click(args: &serde_json::Value) -> Result<ParsedMouseClick, String> {
    let args: MouseClickArgs = serde_json::from_value(args.clone())
        .map_err(|error| format!("invalid mouse_click args: {error}"))?;
    if !(1..=3).contains(&args.count) {
        return Err("mouse_click count must be between 1 and 3".to_string());
    }
    Ok(ParsedMouseClick {
        button: match args.button {
            ButtonArg::Left => MouseButton::Left,
            ButtonArg::Middle => MouseButton::Middle,
            ButtonArg::Right => MouseButton::Right,
            ButtonArg::Back => MouseButton::Back,
            ButtonArg::Forward => MouseButton::Forward,
        },
        count: args.count,
    })
}

struct ParsedMouseClick {
    button: MouseButton,
    count: u8,
}

#[derive(Deserialize)]
struct MouseScrollArgs {
    #[serde(default)]
    delta_x: i32,
    #[serde(default)]
    delta_y: i32,
}

fn parse_mouse_scroll(args: &serde_json::Value) -> Result<MouseScrollArgs, String> {
    let args: MouseScrollArgs = serde_json::from_value(args.clone())
        .map_err(|error| format!("invalid mouse_scroll args: {error}"))?;
    if args.delta_x == 0 && args.delta_y == 0 {
        return Err("mouse_scroll needs a nonzero delta_x or delta_y".to_string());
    }
    if !(-100..=100).contains(&args.delta_x) || !(-100..=100).contains(&args.delta_y) {
        return Err("mouse_scroll deltas must be between -100 and 100".to_string());
    }
    Ok(args)
}

#[derive(Deserialize)]
struct TypeTextArgs {
    text: String,
}

fn parse_type_text(args: &serde_json::Value) -> Result<TypeTextArgs, String> {
    let args: TypeTextArgs = serde_json::from_value(args.clone())
        .map_err(|error| format!("invalid type_text args: {error}"))?;
    let count = args.text.chars().count();
    if count == 0 {
        return Err("type_text text is required".to_string());
    }
    if count > MAX_TEXT_CHARACTERS {
        return Err(format!(
            "type_text text cannot exceed {MAX_TEXT_CHARACTERS} characters"
        ));
    }
    if args.text.contains('\0') {
        return Err("type_text text cannot contain null characters".to_string());
    }
    Ok(args)
}

#[derive(Deserialize)]
struct KeyPressArgs {
    key: String,
    #[serde(default)]
    modifiers: Vec<String>,
    #[serde(default = "default_one")]
    repeat: u8,
}

struct ParsedKeyPress {
    key: ComputerKey,
    label: String,
    modifiers: Vec<ComputerKey>,
    repeat: u8,
}

fn parse_key_press(args: &serde_json::Value) -> Result<ParsedKeyPress, String> {
    let args: KeyPressArgs = serde_json::from_value(args.clone())
        .map_err(|error| format!("invalid key_press args: {error}"))?;
    if !(1..=10).contains(&args.repeat) {
        return Err("key_press repeat must be between 1 and 10".to_string());
    }
    let key = parse_key(&args.key)?;
    let mut seen = HashSet::new();
    let mut modifiers = Vec::with_capacity(args.modifiers.len());
    for value in &args.modifiers {
        let modifier = parse_modifier(value)?;
        if !seen.insert(modifier) {
            return Err(format!("key_press modifier is duplicated: {value}"));
        }
        modifiers.push(modifier);
    }
    Ok(ParsedKeyPress {
        key,
        label: args.key,
        modifiers,
        repeat: args.repeat,
    })
}

fn parse_modifier(value: &str) -> Result<ComputerKey, String> {
    match value.to_ascii_lowercase().as_str() {
        "control" => Ok(ComputerKey::Control),
        "alt" => Ok(ComputerKey::Alt),
        "shift" => Ok(ComputerKey::Shift),
        "meta" => Ok(ComputerKey::Meta),
        _ => Err(format!("unsupported key_press modifier: {value}")),
    }
}

fn parse_key(value: &str) -> Result<ComputerKey, String> {
    if value.contains('\0') {
        return Err("key_press key cannot contain null characters".to_string());
    }
    if value.chars().count() == 1 {
        return Ok(ComputerKey::Unicode(
            value.chars().next().unwrap_or_default(),
        ));
    }
    let normalized = value.to_ascii_lowercase().replace(['-', ' '], "_");
    let key = match normalized.as_str() {
        "enter" | "return" => ComputerKey::Enter,
        "tab" => ComputerKey::Tab,
        "escape" | "esc" => ComputerKey::Escape,
        "backspace" => ComputerKey::Backspace,
        "delete" => ComputerKey::Delete,
        "up" | "arrow_up" => ComputerKey::Up,
        "down" | "arrow_down" => ComputerKey::Down,
        "left" | "arrow_left" => ComputerKey::Left,
        "right" | "arrow_right" => ComputerKey::Right,
        "home" => ComputerKey::Home,
        "end" => ComputerKey::End,
        "page_up" | "pageup" => ComputerKey::PageUp,
        "page_down" | "pagedown" => ComputerKey::PageDown,
        "space" => ComputerKey::Space,
        _ => {
            if let Some(number) = normalized.strip_prefix('f') {
                if let Ok(number) = number.parse::<u8>() {
                    if (1..=12).contains(&number) {
                        return Ok(ComputerKey::Function(number));
                    }
                }
            }
            return Err(format!("unsupported key_press key: {value}"));
        }
    };
    Ok(key)
}

fn modifier_name(key: ComputerKey) -> &'static str {
    match key {
        ComputerKey::Control => "control",
        ComputerKey::Alt => "alt",
        ComputerKey::Shift => "shift",
        ComputerKey::Meta => "meta",
        _ => "unknown",
    }
}

fn default_one() -> u8 {
    1
}

fn coordinate_name(mode: CoordinateMode) -> &'static str {
    match mode {
        CoordinateMode::Absolute => "absolute",
        CoordinateMode::Relative => "relative",
    }
}

fn button_name(button: MouseButton) -> &'static str {
    match button {
        MouseButton::Left => "left",
        MouseButton::Middle => "middle",
        MouseButton::Right => "right",
        MouseButton::Back => "back",
        MouseButton::Forward => "forward",
    }
}
