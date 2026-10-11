use std::sync::{Arc, Mutex};

use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MouseButton {
    Left,
    Middle,
    Right,
    Back,
    Forward,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CoordinateMode {
    Absolute,
    Relative,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ComputerKey {
    Unicode(char),
    Enter,
    Tab,
    Escape,
    Backspace,
    Delete,
    Up,
    Down,
    Left,
    Right,
    Home,
    End,
    PageUp,
    PageDown,
    Space,
    Control,
    Alt,
    Shift,
    Meta,
    Function(u8),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyDirection {
    Press,
    Release,
    Click,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScrollAxis {
    Horizontal,
    Vertical,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Point {
    pub x: i32,
    pub y: i32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Size {
    pub width: i32,
    pub height: i32,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct WindowBounds {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ActiveWindowState {
    pub title: String,
    pub application: String,
    pub process_id: u64,
    pub window_id: String,
    pub bounds: WindowBounds,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct UnavailableState {
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "status", rename_all = "camelCase")]
pub enum Observation<T> {
    Available { value: T },
    Unavailable { reason: String },
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ComputerUiState {
    pub pointer: Observation<Point>,
    pub primary_display: Observation<Size>,
    pub active_window: Observation<ActiveWindowState>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapturedImage {
    pub png: Vec<u8>,
    pub width: u32,
    pub height: u32,
}

pub trait ComputerController: Send {
    fn move_mouse(&mut self, x: i32, y: i32, coordinate: CoordinateMode) -> Result<(), String>;
    fn click(&mut self, button: MouseButton) -> Result<(), String>;
    fn scroll(&mut self, amount: i32, axis: ScrollAxis) -> Result<(), String>;
    fn type_text(&mut self, text: &str) -> Result<(), String>;
    fn key_event(&mut self, key: ComputerKey, direction: KeyDirection) -> Result<(), String>;
    fn ui_state(&mut self) -> Result<ComputerUiState, String>;
    fn capture_primary_display(&mut self) -> Result<CapturedImage, String>;
}

pub type SharedComputerController = Arc<Mutex<Box<dyn ComputerController>>>;
