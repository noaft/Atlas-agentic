# Screen Capture Vision Design

## Goal

Give Atlas a `capture_screen` runtime tool that captures the primary display and
lets the configured model inspect the pixels as an image. This must solve the
current failure mode where Atlas can create a screenshot file but cannot see or
reason about it.

The first slice captures one still image from the primary display. It does not
add user image attachments, region selection, active-window-only capture, OCR,
screen recording, or multi-monitor composition.

## Product Behavior

- Register `capture_screen` with the default Atlas tools.
- The tool accepts an empty object and rejects unknown fields.
- A successful call captures the primary display and returns concise text with
  its dimensions and format plus an in-memory PNG attachment.
- The next model request contains the normal textual tool continuation and the
  captured PNG as actual image input. Atlas can therefore inspect text, icons,
  layout, and other visual content before choosing another tool or answering.
- The latest successful screenshot remains attached to later model requests in
  the same turn until another successful capture replaces it or the turn ends.
- Screenshots are never written to disk and are not persisted in conversation
  history. Internal tool calls and their attachments remain turn-local, as
  existing text tool results do.
- The default target is the primary display. The tool does not promise that the
  mouse pointer is included because native capture behavior varies by platform.

## Permission and Privacy Model

`capture_screen` uses `ToolRisk::ComputerObserve`. Ask and Workspace modes must
request confirmation; Full Access follows existing behavior and skips the Atlas
confirmation without bypassing operating-system protections.

The computer-observation approval reason will explicitly state that visible
desktop information is read and sent to the configured model provider. The tool
summary identifies the scope as `primary_display`. This matters because a screen
can contain passwords, messages, account information, or other sensitive data.

The PNG exists only in process memory and in the HTTPS request to the configured
provider. Atlas must not include base64 image bytes in approval events, textual
tool continuations, streamed deltas, logs, final answers, or saved conversation
messages.

## Architecture

### Capture boundary

Extend the Atlas-owned computer boundary rather than letting the agent or
provider invoke native capture directly:

```text
CaptureScreenTool
  -> SharedComputerController
  -> ComputerController::capture_primary_display()
  -> SystemComputerController native adapter
```

Add a `CapturedImage` domain value containing MIME type, width, height, and
owned bytes. Tests inject it through the existing recording controller, so no
automated test captures the developer's real screen.

The production adapter uses a maintained cross-platform Rust screenshot crate
to enumerate displays, select the native primary display, capture RGBA pixels,
and encode them as PNG in memory. Windows and macOS use their native capture
backends. Linux support in this slice follows the existing desktop-control
scope: a usable X11 session is supported, while unsupported Wayland sessions
return an actionable error rather than invoking external shell utilities.

macOS capture errors must mention Screen Recording permission. Linux errors
must mention the graphical-session/X11 requirement. Windows errors should
preserve the native capture context.

### Tool output boundary

Extend `ToolResult` with provider-neutral attachments. The initial attachment
variant is an image with MIME type, bytes, dimensions, and model detail hint.
Ordinary tools continue returning text with no attachments.

The runtime keeps tool-result text in `ToolExecutionRecord` exactly as today but
does not serialize attachment bytes into the JSON continuation. It stores only
the latest successful image attachment for the current turn and passes a
borrowed provider-neutral image input alongside the continuation text. A newer
successful capture replaces and drops the older bytes. All bytes are dropped
when the turn returns or errors.

### Agent and provider boundary

Add a provider-neutral `ModelInputImage` type and image collection to the model
request path. Text-only requests remain represented as plain text. The agent
boundary gains an image-aware processing path; its default behavior accepts
empty image lists and returns a clear unsupported-input error for nonempty image
lists, while `LocalAgent` forwards images to its `ModelProvider`.

DeepSeek serialization changes only the current user message:

- without images, `content` remains the existing string;
- with an image, `content` becomes an array containing one `text` part followed
  by one `image_url` part;
- the image URL is a `data:image/png;base64,...` value generated inside the
  DeepSeek adapter;
- image detail is `original` so small UI text is not deliberately downsampled
  before provider processing.

History remains text-only because Atlas persists only user/final-assistant
messages. System and assistant messages never carry images. This matches the
DeepSeek Chat Completions vision contract, which accepts image parts in user
messages.

## Size and Validation Limits

- Only `image/png` is produced by `capture_screen`.
- Reject a capture whose width or height exceeds the provider-supported 8192
  pixel boundary rather than silently cropping it.
- Reject an encoded PNG larger than 32 MiB with an actionable error. This keeps
  inline DeepSeek requests inside the documented single-image limit and avoids
  unbounded request memory.
- Reject zero-sized captures and malformed fake output before constructing a
  model request.
- Base64 encoding occurs only inside the DeepSeek request adapter and is not
  retained after request serialization.

These provider limits are enforced at the current adapter boundary. The generic
attachment type itself remains provider-neutral so another provider can choose
its own transport and limits later.

## Error Handling

- Invalid tool arguments fail before authorization or controller locking.
- Missing primary display, denied OS capture permission, unsupported session,
  native capture failure, PNG encoding failure, and provider-size rejection all
  return clear errors to the model through the existing recoverable tool-result
  path.
- A failed capture does not manufacture or attach an image.
- If an earlier screenshot exists and a later capture fails, the older image
  remains the latest successful observation; the textual result history makes
  the later failure explicit.
- A provider that does not support image input returns a clear error instead of
  treating base64 as text.
- Existing tool-call limits continue to count capture calls and failures.

## Testing

Tests must never call the production capture backend.

Add coverage for:

- tool name, empty schema, `ComputerObserve` risk, approval summary, default
  registration, and unknown-argument rejection;
- a recording controller returning a tiny known PNG, with exact dimensions and
  attachment metadata;
- tool-side capture validation for zero dimensions, plus DeepSeek-adapter
  rejection of over-limit dimensions and encoded size;
- runtime forwarding the image to the next model request while keeping base64
  out of textual continuations and approval events;
- replacement semantics when two screenshots occur in one turn;
- dropping screenshot data at turn completion and never adding it to saved
  conversation messages;
- DeepSeek JSON serialization with string content for text-only requests and
  ordered text/image content parts for visual requests;
- provider-unsupported image behavior;
- production adapter selection and error mapping through pure functions or fake
  backend results, without generating a screenshot.

Run the complete core, desktop Rust, frontend, and build verification commands
from `AGENTS.md`. Existing headless tests must still construct the runtime and
enumerate tools without opening a capture session.

## Documentation and Platform Notes

Update `README.md`, `AGENTS.md`, and `VISION.md` to list `capture_screen`, explain
that an approved screenshot is uploaded to the configured provider, document
macOS Screen Recording permission, and retain the Linux X11 limitation. Do not
add Tauri frontend shell, process, accessibility, screen-capture, or filesystem
capabilities; capture remains entirely in the Rust backend.

Primary references:

- DeepSeek Vision guide: <https://api-docs.deepseek.com/guides/vision/>
- DeepSeek Chat Completions schema:
  <https://api-docs.deepseek.com/api/create-chat-completion/>
- Rust screenshot API documentation:
  <https://docs.rs/screenshots/latest/screenshots/struct.Screen.html>

## Out of Scope

- Selecting a monitor, window, or rectangle
- Combining multiple monitors
- User-uploaded image attachments
- Persisting or previewing screenshots in chat history
- OCR as a separate local pipeline
- Accessibility-tree inspection
- Clipboard capture
- Screen recording or continuous streaming
- Wayland portal integration in this first slice
