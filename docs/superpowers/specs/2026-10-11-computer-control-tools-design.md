# Computer Control Tools Design

## Goal

Give Atlas explicit, runtime-controlled tools for interacting with desktop
applications through the mouse and keyboard on Linux, macOS, and Windows. The
tools must use the existing authorization flow, remain provider-agnostic, and
be testable without sending real input during automated tests.

## Scope

This project adds six agent-facing tools:

- `mouse_move`
- `mouse_click`
- `mouse_scroll`
- `type_text`
- `key_press`
- `get_ui_state`

`get_ui_state` returns structured textual metadata: pointer coordinates,
primary-display dimensions, and active-window title, application, process ID,
and bounds when the platform can provide them. A full accessibility tree,
screen capture, OCR, image-model input, drag gestures, clipboard access, and
background input targeted at a specific application are outside this project.

## Tool contracts

`mouse_move` accepts integer `x` and `y` coordinates plus a `coordinate` value
of `absolute` or `relative`, defaulting to `absolute`.

`mouse_click` accepts `button` (`left`, `middle`, `right`, `back`, or
`forward`) and a `count` from one through three. Both fields have safe defaults
of `left` and one.

`mouse_scroll` accepts integer `delta_x` and `delta_y`. At least one delta must
be nonzero. Each delta is limited to 100 wheel steps in either direction.

`type_text` accepts nonempty text of at most 10,000 Unicode scalar values. Its
approval summary exposes a bounded preview and character count rather than the
entire value.

`key_press` accepts one key, an optional unique list of modifiers, and a repeat
count from one through ten. Supported named keys include Enter, Tab, Escape,
Backspace, Delete, arrows, Home, End, Page Up/Down, Space, and F1-F12. A single
Unicode scalar value is also a valid key. Modifiers are `control`, `alt`,
`shift`, and `meta`. The backend releases modifiers in reverse order even when
the main key operation fails.

`get_ui_state` accepts an empty object and returns pretty JSON. Individual
observation fields may contain an `unavailable` explanation rather than making
the entire call fail when, for example, active-window metadata is unavailable.
It fails only when no display connection can be established.

## Architecture

The six `Tool` implementations depend on a shared `ComputerController` trait,
not directly on an OS library. Domain types describe mouse buttons,
coordinates, keys, modifiers, and UI state. A shared controller handle allows
the tools to reuse the same lazily initialized desktop connection.

The production `SystemComputerController` translates domain operations to
Enigo keyboard and mouse operations. Active-window observation uses a narrow
platform adapter. Third-party types do not appear in public Atlas tool schemas
or runtime interfaces. Tests inject a recording fake controller and assert
validated calls without touching the real desktop.

The production controller is lazy. Building an Atlas runtime must continue to
work in a terminal, CI job, SSH session, or service without a graphical
display; only invocation of a computer tool attempts to connect to the active
desktop session.

## Platform behavior

- Windows uses the native input implementation supplied by Enigo. Windows may
  reject input into a process running at a higher integrity level.
- macOS uses Quartz events. Missing Accessibility authorization produces an
  actionable error and the OS may present its permission prompt.
- Linux initially uses X11 through a pure Rust client. A session without a
  usable X11 display returns an actionable unsupported/display-connection
  error. Wayland portal input is intentionally deferred because it has a
  user-mediated session lifecycle that should not be hidden inside a
  synchronous single tool call.

The schemas and controller boundary remain stable when a Wayland portal
session manager is added later.

## Permissions

Two risks are added:

- `ComputerObserve`: reads desktop state that can reveal sensitive context.
- `ComputerControl`: sends input to the active desktop application.

Both risks require confirmation in Ask and Workspace modes. Full Access skips
Atlas confirmation but never bypasses operating-system permissions. Tool
argument validation occurs before confirmation. Approval summaries are
bounded; secret-like JSON fields retain the existing redaction behavior.

The desktop frontend receives approvals through the existing Rust approval
bridge. No shell, process, accessibility, or unrestricted filesystem Tauri
permission is granted to frontend JavaScript.

## Error handling

Invalid buttons, coordinates, counts, deltas, keys, duplicate modifiers, null
characters, or oversized text fail before any input is sent. Controller
failures identify the attempted operation and preserve the underlying platform
error. Multi-step operations stop at the first failed action and attempt to
release held modifiers before returning the error.

Successful results report the completed action without claiming that the
target application handled it. Desktop focus can change between authorization
and execution, so approval copy states that input is sent to the application
active at execution time.

## Testing

Tests are written before production behavior and cover:

- exact tool definitions and registration;
- every argument default, validation limit, and invalid enum value;
- controller calls for absolute/relative movement, each mouse button,
  two-axis scrolling, Unicode text, named/single-character keys, modifiers,
  repeats, and UI state serialization;
- modifier cleanup after a simulated failure;
- `ComputerObserve` and `ComputerControl` confirmation behavior in all three
  permission modes;
- bounded/redacted approval summaries;
- lazy construction so headless runtime creation remains successful; and
- host compilation plus existing core, desktop Rust, and frontend suites.

Tests never generate real mouse or keyboard input.

## Documentation

README and AGENTS document the six tools, their active-application semantics,
platform limitations, and the fact that Full Access does not grant OS-level
Accessibility or desktop-session permission.
