# Native IME Composer Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the chat composer lifecycle with one stationary native textarea that accepts UniKey and other operating-system IMEs without Atlas interrupting composition.

**Architecture:** Extract composer ownership into a small controller that creates its DOM once and only handles native input, sizing, availability, and explicit draft replacement. Refactor the chat view to mount a stable shell once and patch sibling regions without replacing or moving the composer subtree.

**Tech Stack:** TypeScript 7, DOM APIs, Vitest 5 with jsdom, Tauri 2/WebView2

**Spec:** `docs/superpowers/specs/2026-10-10-native-ime-composer-design.md`

## Global Constraints

- System IMEs own all text conversion; do not add JavaScript Telex or Vietnamese-specific transformation.
- The composer textarea must keep the same node, parent, and connection from mount until unmount.
- Unrelated state renders must not write its value, focus or blur it, move it, replace it, or toggle its disabled state.
- Keep multiline input, automatic height, Enter-to-send, Shift+Enter newline, and the existing `isComposing`/key-code-229 submit guard.
- Keep the change inside the desktop frontend; do not change Rust commands, runtime behavior, permissions, or CLI behavior.
- Work directly on `main` without a worktree, as requested by the user.
- Use test-first development and create focused commits after repository-required checks.

## Review Focus

- UniKey emits a composed first character while initialization completes: composition and draft must survive.
- A user edits an existing draft while conversations or models update: value and caret must remain browser-owned.
- A send succeeds or fails: success clears the visible draft and failure restores it without rebuilding the textarea.
- Settings, new-chat, and conversation-selection transitions: the intended draft changes once without stale text.
- Streaming, workspace, and model busy states: submission is blocked when required while the textarea remains editable.

---

### Task 1: Stationary Native Composer Controller

**Files:**
- Create: `desktop/src/chat-composer.ts`
- Create: `desktop/src/chat-composer.test.ts`

**Interfaces:**
- Consumes: `ChatStore`, `Readonly<ChatState>`, and `onSubmit: () => Promise<void>`.
- Produces: `createChatComposer(store, onSubmit): ChatComposer` and `ChatComposer` members `element`, `textarea`, `modelHost`, `feedbackHost`, `sendButton`, `updateState(state)`, `replaceDraft(text)`, `focus()`, and `destroy()`.

- [ ] **Step 1: Write failing native-input ownership tests**

Add tests named `native input updates the draft without transformation`, `state updates never mutate the live textarea`, and `composer stays editable while send availability follows state`. Assert that input containing `á` reaches `store.draftText` unchanged; repeated `updateState` calls preserve textarea identity, parent, value, selection, focus, and enabled state; and only the send button becomes disabled for busy or missing-workspace states.

- [ ] **Step 2: Run the controller tests and verify RED**

Run: `npm --prefix desktop test -- chat-composer.test.ts`

Expected: FAIL because `chat-composer.ts` and `createChatComposer` do not exist.

- [ ] **Step 3: Implement the persistent composer DOM and ordinary input synchronization**

Create `ChatComposer` with one form and textarea constructed once. Handle `input` by resizing and calling `store.setDraft(textarea.value)`. `updateState` may patch placeholder, note, send availability, and model/feedback hosts, but must not assign `textarea.value`, focus/blur it, move it, or disable it. `replaceDraft` is the only method allowed to assign the value and resize.

- [ ] **Step 4: Add failing keyboard and cleanup tests**

Add tests named `composition keys never submit`, `Enter submits and Shift Enter remains native`, and `destroy removes window listeners`. Assert that `isComposing`, key code `229`, and Shift+Enter do not submit; plain Enter submits once; and resize no longer changes height after destruction.

- [ ] **Step 5: Implement guarded submission, resizing, focus, and cleanup**

Keep the key guard local to the stable textarea. The form submit handler must consult current store state and live text, call `onSubmit` only when allowed, and never blur the textarea. Register and remove the window resize listener in the controller lifecycle.

- [ ] **Step 6: Run controller tests and verify GREEN**

Run: `npm --prefix desktop test -- chat-composer.test.ts`

Expected: all composer-controller tests PASS with no warnings or unhandled rejections.

- [ ] **Step 7: Commit the controller**

Run `cargo fmt --check`, `cargo test`, inspect `git status --short` and the staged diff, then commit only the two controller files:

```bash
git add desktop/src/chat-composer.ts desktop/src/chat-composer.test.ts
git commit -m "feat: add persistent native chat composer"
```

### Task 2: Stable Chat View Rendering Boundary

**Files:**
- Modify: `desktop/src/chat-view.ts`
- Modify: `desktop/src/chat-view.test.ts`
- Consume: `desktop/src/chat-composer.ts`

**Interfaces:**
- Consumes: `createChatComposer(store, onSubmit): ChatComposer` from Task 1.
- Produces: a `mountChat` implementation whose shell and composer ownership remain stable while sidebar, messages, dialogs, model controls, and Settings are patched independently.

- [ ] **Step 1: Replace move emulation tests with failing stationary-parent regressions**

Remove test-only `moveBefore` emulation. Add or update tests named `composer node and parent stay mounted across every background render`, `background render does not touch focused composer APIs`, and `composed first character survives initialization`. Spy on the textarea instance's `focus`, `blur`, and value setter plus `Element.prototype.moveBefore`; trigger settings/model/list initialization and streaming updates; assert none is called and the same connected textarea, parent, value `á`, caret, and focus remain.

- [ ] **Step 2: Run view tests and verify RED**

Run: `npm --prefix desktop test -- chat-view.test.ts`

Expected: FAIL because current `root.innerHTML`, `main.innerHTML`, atomic moves, and composition fallback touch or relocate the composer.

- [ ] **Step 3: Mount the shell once and isolate mutable render regions**

In `mountChat`, create the shell, sidebar, chat header, notices, messages, composer host, dialog host, and Settings host once. Append `ChatComposer.element` once. Refactor rendering so updates replace children only inside regions that cannot contain the composer. Hide/show chat and Settings regions without rebuilding the textarea.

- [ ] **Step 4: Remove obsolete IME lifecycle workarounds**

Delete `AtomicMoveElement`, `supportsAtomicMove`, `moveAtomically`, composition flags/timers, pending render state, blur fallback, composer parking, textarea recreation, and render-time focus restoration. Keep streaming message patching only if it still operates entirely outside the composer subtree.

- [ ] **Step 5: Patch controls from current state without stale event closures**

Ensure sidebar, workspace, conversation actions, model picker, notices, messages, dialog, and Settings handlers consult current store state or are recreated only within their isolated regions. Call `composer.updateState(state)` on renders; do not pass draft text through this method.

- [ ] **Step 6: Run view and full frontend tests and verify GREEN**

Run:

```bash
npm --prefix desktop test -- chat-view.test.ts
npm --prefix desktop test
npm --prefix desktop run build
```

Expected: all tests PASS and TypeScript/Vite build succeeds.

- [ ] **Step 7: Commit the stable rendering boundary**

Run `cargo fmt --check`, `cargo test`, inspect `git status --short` and the staged diff, then commit the view integration:

```bash
git add desktop/src/chat-view.ts desktop/src/chat-view.test.ts
git commit -m "fix: keep native composer mounted"
```

### Task 3: Explicit Draft Transitions and Final Verification

**Files:**
- Modify: `desktop/src/chat-view.ts`
- Modify: `desktop/src/chat-view.test.ts`
- Modify if test ownership requires it: `desktop/src/chat-composer.test.ts`

**Interfaces:**
- Consumes: `ChatComposer.replaceDraft(text)` only for deliberate draft transitions.
- Produces: correct visible/store draft synchronization for new chat, selected conversation, Settings round trips, and successful or failed sends.

- [ ] **Step 1: Write failing explicit-transition tests**

Add tests named `new chat and conversation selection replace the draft without replacing the composer`, `Settings round trip preserves the live draft`, `successful send clears the live draft`, and `failed send restores the live draft`. Assert textarea and parent identity remain stable in every case.

- [ ] **Step 2: Run the transition tests and verify RED**

Run: `npm --prefix desktop test -- chat-view.test.ts`

Expected: at least one draft transition fails because general rendering no longer assigns `textarea.value`.

- [ ] **Step 3: Synchronize only deliberate transitions**

After awaited new-chat, conversation-selection, and send actions, call `replaceDraft(store.getState().draftText)` at the event boundary. Settings open/cancel/save must leave the existing live draft untouched. Do not add a general render-time draft assignment.

- [ ] **Step 4: Run complete repository verification**

Run:

```bash
cargo fmt --check
cargo test
cargo clippy --all-targets --all-features -- -D warnings
cargo fmt --manifest-path desktop/src-tauri/Cargo.toml --check
cargo test --manifest-path desktop/src-tauri/Cargo.toml
cargo clippy --manifest-path desktop/src-tauri/Cargo.toml --all-targets --all-features -- -D warnings
npm --prefix desktop test
npm --prefix desktop run build
npm --prefix desktop run tauri -- build --debug --no-bundle
```

Expected: every command exits zero; frontend output reports all tests passing; the debug application is built.

- [ ] **Step 5: Review and commit the completed fix**

Inspect `git status --short`, `git diff --check`, and the staged diff. Commit only task files:

```bash
git add desktop/src/chat-view.ts desktop/src/chat-view.test.ts desktop/src/chat-composer.test.ts
git commit -m "fix: synchronize explicit composer drafts"
```

- [ ] **Step 6: Push for the native acceptance test**

After the automated checks and final review pass, push `main`. On Windows, launch the newly built Tauri app, enable UniKey Telex, type `á` as the first character, continue with `tiếng Việt`, wait through initialization, edit an existing draft, switch UniKey off/on, test Shift+Enter and Enter, and repeat after Settings/new-chat/conversation selection. Record the result explicitly; if it fails, collect the exact failing transition and do not claim native verification.
