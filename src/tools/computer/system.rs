use std::sync::{Arc, Mutex};

use active_win_pos_rs::ActiveWindow;
use enigo::{Axis, Button, Coordinate, Direction, Enigo, Key, Keyboard, Mouse, Settings};

use super::controller::{
    ActiveWindowState, ComputerController, ComputerKey, ComputerUiState, CoordinateMode,
    KeyDirection, MouseButton, Observation, Point, ScrollAxis, SharedComputerController, Size,
    WindowBounds,
};

pub struct SystemComputerController {
    enigo: Option<Enigo>,
}

impl SystemComputerController {
    pub fn new() -> Self {
        Self { enigo: None }
    }

    fn enigo(&mut self) -> Result<&mut Enigo, String> {
        if self.enigo.is_none() {
            let created = Enigo::new(&Settings::default()).map_err(connection_error)?;
            self.enigo = Some(created);
        }
        self.enigo
            .as_mut()
            .ok_or_else(|| "desktop input connection was not initialized".to_string())
    }
}

impl Default for SystemComputerController {
    fn default() -> Self {
        Self::new()
    }
}

pub fn system_computer_controller() -> SharedComputerController {
    Arc::new(Mutex::new(Box::new(SystemComputerController::new())))
}

impl ComputerController for SystemComputerController {
    fn move_mouse(
        &mut self,
        x: i32,
        y: i32,
        coordinate_mode: CoordinateMode,
    ) -> Result<(), String> {
        self.enigo()?
            .move_mouse(x, y, coordinate(coordinate_mode))
            .map_err(|error| format!("failed to move mouse: {error}"))
    }

    fn click(&mut self, button: MouseButton) -> Result<(), String> {
        self.enigo()?
            .button(mouse_button(button), Direction::Click)
            .map_err(|error| format!("failed to click mouse: {error}"))
    }

    fn scroll(&mut self, amount: i32, axis: ScrollAxis) -> Result<(), String> {
        self.enigo()?
            .scroll(amount, scroll_axis(axis))
            .map_err(|error| format!("failed to scroll mouse: {error}"))
    }

    fn type_text(&mut self, text: &str) -> Result<(), String> {
        self.enigo()?
            .text(text)
            .map_err(|error| format!("failed to type text: {error}"))
    }

    fn key_event(&mut self, input: ComputerKey, direction: KeyDirection) -> Result<(), String> {
        self.enigo()?
            .key(key(input)?, key_direction(direction))
            .map_err(|error| format!("failed to send key event: {error}"))
    }

    fn ui_state(&mut self) -> Result<ComputerUiState, String> {
        let pointer = {
            let enigo = self.enigo()?;
            enigo
                .location()
                .map(|(x, y)| Point { x, y })
                .map(available)
                .unwrap_or_else(|error| unavailable(error.to_string()))
        };
        #[cfg(target_os = "linux")]
        let primary_display = linux_primary_display()
            .map(available)
            .unwrap_or_else(unavailable);
        #[cfg(not(target_os = "linux"))]
        let primary_display = {
            self.enigo()?
                .main_display()
                .map(|(width, height)| Size { width, height })
                .map(available)
                .unwrap_or_else(|error| unavailable(error.to_string()))
        };
        let active_window = active_win_pos_rs::get_active_window()
            .map(active_window_state)
            .map(available)
            .unwrap_or_else(|()| unavailable("active window metadata is unavailable"));

        Ok(ComputerUiState {
            pointer,
            primary_display,
            active_window,
        })
    }
}

#[cfg(target_os = "linux")]
fn linux_primary_display() -> Result<Size, String> {
    use x11rb::connection::Connection as _;
    use x11rb::protocol::randr::ConnectionExt as _;

    let (connection, screen_index) = x11rb::connect(None)
        .map_err(|error| format!("failed to connect to X11 for display observation: {error}"))?;
    let root = connection
        .setup()
        .roots
        .get(screen_index)
        .ok_or_else(|| "X11 did not provide the selected screen".to_string())?
        .root;
    let resources = connection
        .randr_get_screen_resources_current(root)
        .map_err(|error| format!("failed to request X11 display resources: {error}"))?
        .reply()
        .map_err(|error| format!("failed to read X11 display resources: {error}"))?;
    let primary_output = connection
        .randr_get_output_primary(root)
        .map_err(|error| format!("failed to request the X11 primary output: {error}"))?
        .reply()
        .map_err(|error| format!("failed to read the X11 primary output: {error}"))?
        .output;
    if primary_output == x11rb::NONE {
        return primary_display_size(None, &[], &[]);
    }
    let output = connection
        .randr_get_output_info(primary_output, resources.config_timestamp)
        .map_err(|error| format!("failed to request X11 primary-output details: {error}"))?
        .reply()
        .map_err(|error| format!("failed to read X11 primary-output details: {error}"))?;
    if output.crtc == x11rb::NONE {
        return primary_display_size(Some(primary_output), &[(primary_output, 0)], &[]);
    }
    let crtc = connection
        .randr_get_crtc_info(output.crtc, resources.config_timestamp)
        .map_err(|error| format!("failed to request X11 primary-display geometry: {error}"))?
        .reply()
        .map_err(|error| format!("failed to read X11 primary-display geometry: {error}"))?;

    primary_display_size(
        Some(primary_output),
        &[(primary_output, output.crtc)],
        &[(output.crtc, crtc.width, crtc.height)],
    )
}

#[cfg(any(target_os = "linux", test))]
fn primary_display_size(
    primary_output: Option<u32>,
    output_crtcs: &[(u32, u32)],
    crtc_sizes: &[(u32, u16, u16)],
) -> Result<Size, String> {
    let output = primary_output.ok_or_else(|| "X11 has no primary output".to_string())?;
    let crtc = output_crtcs
        .iter()
        .find_map(|(candidate, crtc)| (*candidate == output).then_some(*crtc))
        .filter(|crtc| *crtc != 0)
        .ok_or_else(|| "X11 primary output has no active CRTC".to_string())?;
    let (_, width, height) = crtc_sizes
        .iter()
        .find(|(candidate, _, _)| *candidate == crtc)
        .copied()
        .filter(|(_, width, height)| *width > 0 && *height > 0)
        .ok_or_else(|| "X11 primary output has no usable display geometry".to_string())?;
    Ok(Size {
        width: i32::from(width),
        height: i32::from(height),
    })
}

fn available<T>(value: T) -> Observation<T> {
    Observation::Available { value }
}

fn unavailable<T>(reason: impl Into<String>) -> Observation<T> {
    Observation::Unavailable {
        reason: reason.into(),
    }
}

fn connection_error(error: enigo::NewConError) -> String {
    #[cfg(target_os = "linux")]
    {
        format!("failed to connect to the desktop input session: {error}; Linux requires a usable X11 DISPLAY")
    }
    #[cfg(not(target_os = "linux"))]
    {
        format!("failed to connect to the desktop input session: {error}")
    }
}

fn mouse_button(button: MouseButton) -> Button {
    match button {
        MouseButton::Left => Button::Left,
        MouseButton::Middle => Button::Middle,
        MouseButton::Right => Button::Right,
        MouseButton::Back => Button::Back,
        MouseButton::Forward => Button::Forward,
    }
}

fn coordinate(mode: CoordinateMode) -> Coordinate {
    match mode {
        CoordinateMode::Absolute => Coordinate::Abs,
        CoordinateMode::Relative => Coordinate::Rel,
    }
}

fn scroll_axis(axis: ScrollAxis) -> Axis {
    match axis {
        ScrollAxis::Horizontal => Axis::Horizontal,
        ScrollAxis::Vertical => Axis::Vertical,
    }
}

fn key_direction(direction: KeyDirection) -> Direction {
    match direction {
        KeyDirection::Press => Direction::Press,
        KeyDirection::Release => Direction::Release,
        KeyDirection::Click => Direction::Click,
    }
}

fn key(input: ComputerKey) -> Result<Key, String> {
    let key = match input {
        ComputerKey::Unicode(value) => Key::Unicode(value),
        ComputerKey::Enter => Key::Return,
        ComputerKey::Tab => Key::Tab,
        ComputerKey::Escape => Key::Escape,
        ComputerKey::Backspace => Key::Backspace,
        ComputerKey::Delete => Key::Delete,
        ComputerKey::Up => Key::UpArrow,
        ComputerKey::Down => Key::DownArrow,
        ComputerKey::Left => Key::LeftArrow,
        ComputerKey::Right => Key::RightArrow,
        ComputerKey::Home => Key::Home,
        ComputerKey::End => Key::End,
        ComputerKey::PageUp => Key::PageUp,
        ComputerKey::PageDown => Key::PageDown,
        ComputerKey::Space => Key::Space,
        ComputerKey::Control => Key::Control,
        ComputerKey::Alt => Key::Alt,
        ComputerKey::Shift => Key::Shift,
        ComputerKey::Meta => Key::Meta,
        ComputerKey::Function(number) => match number {
            1 => Key::F1,
            2 => Key::F2,
            3 => Key::F3,
            4 => Key::F4,
            5 => Key::F5,
            6 => Key::F6,
            7 => Key::F7,
            8 => Key::F8,
            9 => Key::F9,
            10 => Key::F10,
            11 => Key::F11,
            12 => Key::F12,
            _ => return Err(format!("unsupported function key: F{number}")),
        },
    };
    Ok(key)
}

fn active_window_state(window: ActiveWindow) -> ActiveWindowState {
    ActiveWindowState {
        title: window.title,
        application: window.app_name,
        process_id: window.process_id,
        window_id: window.window_id,
        bounds: WindowBounds {
            x: window.position.x,
            y: window.position.y,
            width: window.position.width,
            height: window.position.height,
        },
    }
}

#[cfg(test)]
mod tests {
    use active_win_pos_rs::{ActiveWindow, WindowPosition};
    use enigo::{Axis, Button, Coordinate, Key};

    use super::{
        active_window_state, coordinate, key, mouse_button, primary_display_size, scroll_axis,
        SystemComputerController,
    };
    use crate::tools::computer::controller::{
        ComputerKey, CoordinateMode, MouseButton, ScrollAxis,
    };

    #[test]
    fn maps_platform_neutral_mouse_values() {
        assert_eq!(mouse_button(MouseButton::Left), Button::Left);
        assert_eq!(mouse_button(MouseButton::Middle), Button::Middle);
        assert_eq!(mouse_button(MouseButton::Right), Button::Right);
        assert_eq!(mouse_button(MouseButton::Back), Button::Back);
        assert_eq!(mouse_button(MouseButton::Forward), Button::Forward);
        assert_eq!(coordinate(CoordinateMode::Absolute), Coordinate::Abs);
        assert_eq!(coordinate(CoordinateMode::Relative), Coordinate::Rel);
        assert_eq!(scroll_axis(ScrollAxis::Horizontal), Axis::Horizontal);
        assert_eq!(scroll_axis(ScrollAxis::Vertical), Axis::Vertical);
    }

    #[test]
    fn maps_every_supported_keyboard_value() {
        let cases = [
            (ComputerKey::Unicode('é'), Key::Unicode('é')),
            (ComputerKey::Enter, Key::Return),
            (ComputerKey::Tab, Key::Tab),
            (ComputerKey::Escape, Key::Escape),
            (ComputerKey::Backspace, Key::Backspace),
            (ComputerKey::Delete, Key::Delete),
            (ComputerKey::Up, Key::UpArrow),
            (ComputerKey::Down, Key::DownArrow),
            (ComputerKey::Left, Key::LeftArrow),
            (ComputerKey::Right, Key::RightArrow),
            (ComputerKey::Home, Key::Home),
            (ComputerKey::End, Key::End),
            (ComputerKey::PageUp, Key::PageUp),
            (ComputerKey::PageDown, Key::PageDown),
            (ComputerKey::Space, Key::Space),
            (ComputerKey::Control, Key::Control),
            (ComputerKey::Alt, Key::Alt),
            (ComputerKey::Shift, Key::Shift),
            (ComputerKey::Meta, Key::Meta),
            (ComputerKey::Function(1), Key::F1),
            (ComputerKey::Function(2), Key::F2),
            (ComputerKey::Function(3), Key::F3),
            (ComputerKey::Function(4), Key::F4),
            (ComputerKey::Function(5), Key::F5),
            (ComputerKey::Function(6), Key::F6),
            (ComputerKey::Function(7), Key::F7),
            (ComputerKey::Function(8), Key::F8),
            (ComputerKey::Function(9), Key::F9),
            (ComputerKey::Function(10), Key::F10),
            (ComputerKey::Function(11), Key::F11),
            (ComputerKey::Function(12), Key::F12),
        ];

        for (input, expected) in cases {
            assert_eq!(key(input).unwrap(), expected);
        }
    }

    #[test]
    fn construction_is_lazy_and_active_window_conversion_is_complete() {
        let controller = SystemComputerController::new();
        assert!(controller.enigo.is_none());

        let state = active_window_state(ActiveWindow {
            title: "Notes".to_string(),
            app_name: "Editor".to_string(),
            process_id: 42,
            window_id: "window-7".to_string(),
            process_path: "/apps/editor".into(),
            position: WindowPosition::new(10.5, 20.25, 800.0, 600.0),
        });

        assert_eq!(state.title, "Notes");
        assert_eq!(state.application, "Editor");
        assert_eq!(state.process_id, 42);
        assert_eq!(state.window_id, "window-7");
        assert_eq!(state.bounds.x, 10.5);
        assert_eq!(state.bounds.y, 20.25);
        assert_eq!(state.bounds.width, 800.0);
        assert_eq!(state.bounds.height, 600.0);
    }

    #[test]
    fn primary_display_selects_the_primary_outputs_active_crtc() {
        let size = primary_display_size(
            Some(20),
            &[(10, 100), (20, 200)],
            &[(100, 1280, 720), (200, 2560, 1440)],
        )
        .unwrap();

        assert_eq!(size.width, 2560);
        assert_eq!(size.height, 1440);
    }

    #[test]
    fn primary_display_rejects_missing_or_inactive_results() {
        assert!(primary_display_size(None, &[], &[]).is_err());
        assert!(primary_display_size(Some(20), &[], &[]).is_err());
        assert!(primary_display_size(Some(20), &[(20, 0)], &[]).is_err());
        assert!(primary_display_size(Some(20), &[(20, 200)], &[]).is_err());
    }

    #[test]
    fn shared_controller_is_safe_to_move_between_runtime_threads() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<crate::tools::computer::SharedComputerController>();
    }
}
