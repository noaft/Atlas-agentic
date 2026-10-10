# Native IME Composer Design

## Goal

Make the Atlas desktop chat composer accept text through operating-system input
methods such as UniKey in the same way as ordinary browser and Settings inputs.
Atlas must not implement, force, or transform Telex itself. Users can enable or
disable their preferred system input method without changing Atlas settings.

Success means a Windows user can type Vietnamese characters such as `á`, continue
typing while Atlas updates other UI, edit an existing draft, and switch UniKey on
or off without lost characters, frozen composition, unexpected conversion, or
caret movement.

## Evidence and Root Cause

UniKey works in Atlas Settings and in a normal browser, so the native input method
and WebView are functional. The failure is isolated to the chat composer.

Unlike Settings inputs, the composer currently participates in full-root HTML
replacement and contains custom composition timers, DOM parking and reinsertion,
programmatic focus restoration, value assignment, and busy-state disabling. These
operations can reset or invalidate the native IME context. Preserving a node with
an atomic DOM move is still a move and does not make the composer equivalent to a
normal, stationary input.

## Design

### Permanent composer host

`mountChat` will create the composer form and native `<textarea>` once. The
textarea will remain attached to the same parent until the chat view is unmounted.
Rendering conversation history, loading state, sidebar state, model state, and
streaming output will update other DOM regions without replacing, moving, or
recreating the composer.

The Settings screen may hide the chat shell, but opening and closing Settings must
not rebuild the composer. A new chat or selected conversation updates the draft
only through an explicit conversation transition, not through general rendering.

### Native input ownership

The textarea will use normal browser `input` events to update `ChatStore.draftText`.
Atlas will remove custom `compositionstart`, `compositionend`, composition-settle
timers, atomic movement, and fallback redraw logic from the composer. No JavaScript
Telex conversion or Vietnamese-specific transformation will be introduced.

Atlas will not assign `textarea.value`, change focus, or change the disabled state
in response to unrelated store updates while the user is editing. Send availability
will be derived from current store state and the live textarea value without
rewriting that value.

### Keyboard and sending behavior

The composer remains multiline. Shift+Enter inserts a newline. Enter submits only
when the keyboard event is not composing and does not have the Windows IME process
key code `229`. Submission reads the live textarea value, lets the existing store
send flow run, and synchronizes the cleared or retained draft after the operation.

The textarea stays editable while background initialization and unrelated UI
updates occur. Busy state prevents submission through the Send button and submit
handler instead of disabling or replacing the textarea. Automatic height remains
local to ordinary input and window-resize events.

### Rendering boundary

The chat view will have stable top-level regions for navigation, header/messages,
composer, dialogs, and Settings. Store updates patch the relevant regions. The
implementation may keep existing rendering helpers, but no path that handles an
unrelated state change may write through a container that owns the composer.

This boundary is intentionally limited to the desktop view. It does not change the
conversation store, Rust commands, provider runtime, native permissions, or CLI.

## Error and Edge-Case Behavior

- Without a valid workspace, users may type a draft but cannot submit it.
- While Atlas is sending or changing workspace/model state, the draft remains
  editable and the submit action remains guarded by current store state.
- Selecting a conversation or starting a new chat intentionally replaces the draft
  once, outside an active composition. If a composition is active, the transition
  is deferred until composition completes or the textarea loses focus.
- Opening Settings does not discard the draft; returning to chat reveals the same
  textarea and value.
- Failed sends preserve the store's existing retry and draft semantics.

## Testing

Frontend regression tests will prove that:

1. The exact textarea node and parent remain unchanged across initialization,
   sidebar, model, conversation-list, and streaming updates.
2. Background renders do not invoke `focus`, `blur`, `moveBefore`, or value writes
   on the composer.
3. Composition input is stored without transformation, including a composed `á`.
4. A composition survives a background update and remains sendable afterward.
5. Enter is ignored during composition; Enter and Shift+Enter retain their current
   post-composition behavior.
6. New-chat, conversation-selection, Settings, send-success, and send-failure draft
   transitions remain correct.
7. Automatic textarea height still responds to input and window resize.

The full Rust, Tauri, frontend, lint, and desktop debug-build checks must pass. A
manual Windows smoke test with UniKey is required because jsdom cannot validate the
Windows TSF context itself.
