# MCP client implementation plan

**Goal:** Let Atlas use tools from user-configured local MCP servers in both CLI and desktop.

**Architecture:** Keep MCP behind the existing runtime tool boundary. Register three gateway tools only when servers are configured: list configured servers without launching, discover a server's tools, and call a tool. Discovery and calls require approval in Ask and Workspace because they may launch arbitrary configured processes or perform external actions. Full access preserves existing semantics.

**Scope:** Stdio JSON-RPC, the 2025-11-25 initialization handshake (also accepting 2025-06-18 and 2025-03-26), paginated discovery, text and structured results. HTTP, OAuth, resources, prompts, sampling, and binary content are outside this first version. Config is user-local, never read from an untrusted project directory. Processes inherit the Atlas environment and starting directory. No automatic retries of failed tool calls.

## Tasks

- [x] Write failing configuration and subprocess behavior tests using an offline fake MCP server.
- [x] Implement validated `mcp.toml` configuration, bounded stdio transport, initialization, timeouts and cleanup.
- [x] Implement gateway tools and shared-runtime registration, with permission and end-to-end denial tests.
- [x] Document setup, supported behavior and limitations. Run formatting, Rust tests and clippy; verify desktop backend compilation where available.
- [x] Review the diff and create one focused commit, per repository instructions.

## Review focus

Never launch on registration or argument validation. Reject unknown servers, malformed envelopes, unsupported versions and oversized frames. Bound pagination and request duration. Do not trust remote read-only annotations, echo server protocol errors or stderr into backend errors, or persist binary payloads. Close failed sessions without replaying calls; terminate owned processes on drop.

## Validation

Root and desktop Rust formatting, full tests and clippy passed. Desktop UI tests (212 tests), production frontend build and Tauri debug build passed. Twelve MCP behavior tests cover the new boundary, with two additional CLI integration tests. Independent review identified Windows descendant cleanup and loss of corrective tool-error details; both are addressed, with the tool-error regression observed failing before the fix and passing afterward. Windows uses a suspended launch and kill-on-close Job Object. The exact Windows helper cross-compiles, but the full Windows check is blocked by the environment's missing `x86_64-w64-mingw32-gcc`; its native descendant-cleanup test still requires Windows execution.
