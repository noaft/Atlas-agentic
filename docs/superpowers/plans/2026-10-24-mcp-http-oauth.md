# HTTP MCP and OAuth implementation plan

**Goal:** Connect Atlas to remote HTTP MCP servers with an explicit browser sign-in flow in Settings and tokens stored by the OS credential service.

**Architecture:** Extend the existing three MCP gateways with a transport boundary, preserving local stdio configuration. HTTP uses Streamable HTTP, JSON or bounded SSE replies, negotiated protocol/session headers and existing tool approvals. OAuth authorization runs only from explicit Connect actions. Rust owns metadata discovery, public-client registration or a configured client ID, state and S256 PKCE, a bounded loopback callback, token exchange/refresh, secure token storage and cancellation. Frontend gets connection status, never tokens or authorization codes.

**Scope:** Authorization Code with PKCE for public desktop clients. Pre-registered client IDs and dynamic registration are supported; no embedded client secrets, silent browser windows, plaintext token fallback, client-metadata hosting or machine-to-machine grants. Authenticated MCP POSTs are not replayed after failure. Disconnect forgets local credentials; service-side grant revocation remains with that service.

## Tasks

- [x] HTTP/config: failing offline tests; validate mutually exclusive command/URL configurations; implement JSON/SSE transport and integrate gateways.
- [x] OAuth: failing offline tests; protected-resource/issuer discovery and public-client registration; PKCE/state callback; keyring token storage, refresh and cancellation.
- [x] Desktop: background Tauri commands and typed API; Settings status/connect/cancel/disconnect without disturbing model drafts; focused UI tests.
- [x] Integrate and review boundaries; document configuration; run all repository checks and create one focused commit.

## Review focus

Keep redirects disabled and credentials bound to their resource/client. Require HTTPS for authorization endpoints, exact issuer/resource checks and S256 support. Never put codes/tokens/verifiers in errors, diagnostics, model context, frontend state or plaintext files. Bound HTTP bodies, callback headers and wait duration. Wrong-state callbacks, concurrent refresh, cancel/disconnect races and expired/rejected tokens must not publish stale credentials or replay tool actions.

## Verification and review

All 385 root Rust, 52 desktop Rust and 222 frontend tests passed. Both Rust formatting and clippy checks passed; the frontend production build and Tauri debug application build passed. Offline tests cover the complete preregistered-client login, PKCE/state/issuer checks, discovery and public-client registration, refresh rotation and rejection, shared credential leases, cancellation before worker startup and during keyring save, cross-instance disconnect invalidation, malformed credential recovery, JSON/SSE HTTP replies, sessions, timeouts and non-replay. Independent review found cancellation-during-save, open discovery SSE and hidden recovery controls; regression tests demonstrated the failures before their fixes. No real provider authorization or OS credential write was performed; tests use an injected in-memory store. Linux builds are verified; Windows and macOS sign-in runtime checks remain for those platforms.

HTTP does not maintain a persistent GET stream, resume streams, automatically restart a failed session, issue session DELETE, or execute server-initiated requests. These limitations are documented in README alongside configuration and sign-in steps.
