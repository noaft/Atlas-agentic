# Computer Control Tools Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add six authorized, cross-platform Atlas tools for mouse movement, mouse clicks, scrolling, text entry, key presses, and textual desktop-state observation.

**Architecture:** Agent-facing tools depend on an Atlas-owned `ComputerController` trait through one shared controller handle. A lazy production adapter translates domain operations to Enigo and active-window inspection, while tests inject a recording fake so no automated test sends real input.

**Tech Stack:** Rust, serde/serde_json, Enigo 0.6, active-win-pos-rs 0.11, existing Atlas runtime authorization and tool registry.

**Spec:** `docs/superpowers/specs/2026-10-11-computer-control-tools-design.md`

## Global Constraints

- Register exactly `mouse_move`, `mouse_click`, `mouse_scroll`, `type_text`, `key_press`, and `get_ui_state` as default tools.
- Support Windows and macOS through Enigo and Linux X11 through Enigo's `x11rb` backend; unsupported graphical sessions return actionable errors.
- Runtime construction remains successful without a graphical session; the production connection is lazy.
- `ComputerObserve` and `ComputerControl` require approval in Ask and Workspace modes and run without Atlas confirmation only in Full Access.
- Input goes to the application active at execution time and never claims the target consumed it.
- Tests never invoke the production controller or generate real input.
- Do not add Tauri frontend shell, process, accessibility, or filesystem capabilities.
- Preserve unrelated work and finish with one focused implementation commit after folding the already committed design into it.

## Review Focus

- A `key_press` failure after pressing modifiers must release every pressed modifier in reverse order; Task 3 tests the failure event sequence.
- Duplicate modifiers must fail validation before locking or calling the controller; Task 3 asserts zero recorded events.
- Text containing a null scalar or more than 10,000 scalar values must fail before authorization/execution while multibyte text counts by scalars, not bytes; Task 3 covers all three cases.
- A headless process must build a runtime and list tool definitions without attempting an Enigo connection; Tasks 2 and 3 test lazy construction and registration.
- UI observation with available pointer/display data but unavailable active-window metadata must return useful partial JSON; Tasks 2 and 3 test the partial state.

---

### Task 1: Computer permission risks

**Files:**
- Modify: `src/permissions.rs`
- Modify: `tests/permission_behavior.rs`

**Interfaces:**
- Produces: `ToolRisk::ComputerObserve` and `ToolRisk::ComputerControl`.
- Produces: risk reasons describing sensitive desktop-state reading and input sent to the active application.
- Produces: `requires_confirmation` behavior that confirms both risks in Ask and Workspace and bypasses confirmation only in Full Access.

- [ ] **Step 1: Extend the permission matrix test**

Add both computer risks to `permission_matrix_matches_the_three_modes`, asserting confirmation in Ask and Workspace and no confirmation in Full Access. Add exact reason assertions for both variants.

- [ ] **Step 2: Run the focused test to verify RED**

Run: `cargo test --test permission_behavior`

Expected: compilation fails because the two variants do not exist.

- [ ] **Step 3: Implement the permission variants and policy**

Add both enum variants, their exact reason text, and both variants to the Workspace confirmation match in `src/permissions.rs`.

- [ ] **Step 4: Run the focused test to verify GREEN**

Run: `cargo test --test permission_behavior`

Expected: all permission tests pass.

### Task 2: Controller boundary and production adapter

**Files:**
- Create: `src/tools/computer/mod.rs`
- Create: `src/tools/computer/controller.rs`
- Create: `src/tools/computer/system.rs`
- Modify: `src/tools/mod.rs`
- Modify: `Cargo.toml`
- Modify: `Cargo.lock`
- Test: `src/tools/computer/system.rs`

**Interfaces:**
- Produces: domain enums `MouseButton`, `CoordinateMode`, `ComputerKey`, `KeyDirection`, and `ScrollAxis`.
- Produces: serializable `Point`, `Size`, `WindowBounds`, `ActiveWindowState`, `UnavailableState`, and `ComputerUiState`, with observation fields represented as available values or actionable unavailable messages.
- Produces: `trait ComputerController` methods `move_mouse`, `click`, `scroll`, `type_text`, `key_event`, and `ui_state`, all returning `Result<_, String>`.
- Produces: `type SharedComputerController = Arc<Mutex<Box<dyn ComputerController>>>` and `system_computer_controller() -> SharedComputerController` without opening a display connection.
- Produces: `SystemComputerController` with lazy `Enigo::new`, domain-to-Enigo translation, active-window inspection, and operation-specific error context.

- [ ] **Step 1: Add controller mapping and lazy-construction tests**

Unit-test every domain-to-Enigo button, coordinate, axis, named key, modifier, and F-key mapping as pure functions. Test that `SystemComputerController::new()` has no initialized Enigo connection. Test active-window conversion separately from OS inspection, including title, application, PID, and bounds.

- [ ] **Step 2: Run the controller tests to verify RED**

Run: `cargo test --lib tools::computer::system::tests`

Expected: compilation fails because the computer module and dependencies do not exist.

- [ ] **Step 3: Implement domain types, shared controller, and system adapter**

Use target-specific Enigo dependency configuration: no default features on Windows/macOS, and `x11rb` on Linux. Keep `active-win-pos-rs` target-neutral. Store `Option<Enigo>` and initialize it only inside controller operations. Convert active-window lookup failure into an unavailable field after pointer/display observation succeeds.

- [ ] **Step 4: Run controller tests to verify GREEN**

Run: `cargo test --lib tools::computer::system::tests`

Expected: all mapping and lazy-construction tests pass without sending input.

### Task 3: Six agent-facing tools and default registration

**Files:**
- Create: `src/tools/computer/input_tools.rs`
- Create: `src/tools/computer/ui_state_tool.rs`
- Modify: `src/tools/computer/mod.rs`
- Modify: `src/tools/mod.rs`
- Modify: `src/app.rs`
- Create: `tests/computer_tools_behavior.rs`
- Modify: `tests/agents_behavior.rs`

**Interfaces:**
- Consumes: Task 1 risks and Task 2 controller/domain types.
- Produces: `MouseMoveTool`, `MouseClickTool`, `MouseScrollTool`, `TypeTextTool`, `KeyPressTool`, and `GetUiStateTool`, each with `new(SharedComputerController)`.
- Produces: parsers enforcing coordinate/button defaults, click range 1..=3, scroll range -100..=100 with a nonzero axis, text range 1..=10,000 scalars with no null, supported named/single-scalar keys, unique modifiers, and repeat range 1..=10.
- Produces: `build_default_tools` registration using one shared lazy system controller for all six tools.

- [ ] **Step 1: Write failing behavior tests with a recording controller**

Test exact names, descriptions, schemas, risk values, defaults, every validation boundary, all mouse buttons, both coordinate modes, two-axis scroll order, Unicode text, named and character keys, modifiers, repeats, structured UI-state JSON, partial unavailable active-window data, and concise result messages. The fake records calls and can fail a selected key event.

- [ ] **Step 2: Add failure-cleanup and privacy-summary tests**

Assert modifiers press in request order and release in reverse order after success and after a simulated main-key failure. Assert duplicate modifiers generate no controller events. Assert `type_text` summaries include scalar count and only a bounded preview, with secret-like nested fields still handled by the existing generic redactor.

- [ ] **Step 3: Run focused tests to verify RED**

Run: `cargo test --test computer_tools_behavior --test agents_behavior`

Expected: compilation/tests fail because tools and default registrations do not exist.

- [ ] **Step 4: Implement the six tools and parsers**

Keep input parsing and sequencing in `input_tools.rs`; keep state serialization in `ui_state_tool.rs`. Acquire the controller mutex once per tool execution. On poisoned locks, return `computer controller lock was poisoned`. For `key_press`, always attempt cleanup and return the primary failure unless only cleanup failed.

- [ ] **Step 5: Register the tools and update exact-name documentation assertions**

Construct one `system_computer_controller()` in `build_default_tools`, clone the shared handle into all six tools, and update the default-tool name test and README-backed agent behavior assertion.

- [ ] **Step 6: Run focused and core tests to verify GREEN**

Run: `cargo test --test computer_tools_behavior --test agents_behavior && cargo test`

Expected: all tests pass without real desktop input.

### Task 4: Documentation, platform compilation, and final verification

**Files:**
- Modify: `README.md`
- Modify: `AGENTS.md`
- Modify: `VISION.md`

**Interfaces:**
- Consumes: the completed tool schemas, permission behavior, and platform limitations.
- Produces: user-facing documentation for all six tools, active-application semantics, Ask/Workspace confirmation, OS permission requirements, Linux X11 limitation, and Full Access boundaries.

- [ ] **Step 1: Update repository documentation**

Add the six tools to the built-in tool list and describe observation/control approvals. Document Windows integrity limits, macOS Accessibility permission, Linux X11 support with actionable Wayland errors, and that no frontend Tauri capability was broadened.

- [ ] **Step 2: Run formatting and all required suites**

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
```

Expected: every command succeeds.

- [ ] **Step 3: Review the complete diff and repository status**

Run: `git diff --check && git status --short && git diff HEAD~1 --stat`

Expected: only design, plan, computer-tool implementation, dependency lockfile, tests, and documentation are changed.

- [ ] **Step 4: Create one focused traceability commit**

Fold the design commit and all implementation work into one final commit with message `feat: add computer control tools`, without including unrelated files.
