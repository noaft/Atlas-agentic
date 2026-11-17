# MCP implementation contracts

Implemented Rust backend contracts for `src/mcp/` and `src/mcp/oauth/`.
See the [MCP user guide](../guides/reference.md#mcp-connections) for configuration.

## Configuration and transport

User-local `mcp.toml` adds optional `mcp_list_servers`, `mcp_list_tools` and
`mcp_call_tool` gateways, shared by CLI and desktop. `src/mcp/` owns validated
stdio/HTTP configuration, JSON-RPC transports, negotiation, discovery and
result conversion. `ATLAS_MCP_CONFIG` overrides the file; otherwise load it beside
settings using `ATLAS_CONFIG_DIR` or the platform user configuration directory.
Never discover configuration from the workspace or launch processes during
runtime construction or authorization validation. Server discovery and calls
start connections only during tool execution, after existing runtime approval.
Ask and Workspace require confirmation for both; external calls use
`ExternalTool` risk regardless of remote annotations. Full access preserves
existing semantics. No frontend permissions are added.
Local servers inherit Atlas's environment and workspace. Preserve bounded requests,
pagination and protocol framing; never automatically replay failed calls.
Timeouts have unknown action outcomes. Keep arbitrary server stderr and protocol
error bodies out of backend errors/logs. Only text and structured tool results
are supported; reject non-text content without persisting image payloads.

## HTTP and OAuth

HTTP uses Streamable HTTP with bounded JSON/SSE replies and negotiated session
and protocol headers. Disable redirects and automatic retries, including after
authentication failure or an expired session. Keep OAuth entirely in Rust under
`src/mcp/oauth/`: explicit desktop Settings Connect/Cancel/Disconnect commands
run on background workers; chat requests never initiate browser authorization.
Use protected-resource and issuer metadata discovery, public-client registration
or a configured public client ID, state and S256 PKCE, a bounded literal-loopback
callback, resource/client-bound credentials and OS credential storage. OAuth
endpoints require HTTPS; no plaintext credential fallback, client secret or
frontend native permissions. Never expose codes/tokens/verifiers to model
context, diagnostics or frontend state. Preserve useful per-server storage errors
and Disconnect recovery controls without hiding other servers. Refresh under
process and file leases, including refresh rotation; disconnect increments a
nonsecret invalidation generation under the same lease so pending sign-ins from
other processes cannot republish credentials. Cancel during a keyring write must
remove any late save. Browser sign-in cancellation and deadlines must not publish
late network results. The Settings MCP panel owns asynchronous status updates
without remounting model inputs or disturbing their drafts/focus/scroll.

## Settings configuration and tool checks

Settings may add new servers through the bounded user-local McpConfigStore:
preserve original bytes/comments, reject duplicate names and malformed/unsupported
or symlink/special-file inputs, and bind writes to a path/content fingerprint under
a bounded file lease with private atomic publication. Never overwrite existing
server identities or probe on load/save. Explicit Check tools uses the existing
MCP registry authorization and discovery with Settings-owned operation IDs,
approvals and cancellation. Saved conversation workspace and permission mode are
native authority. Check cancellation stops at the next RPC boundary; an in-flight
RPC remains bounded to 30 seconds. Retain Cancelling state until its worker exits;
ignore stale approvals/results and reject pre-save refresh responses after a save.
Keep sign-in status distinct from checked tool availability. Config/context changes
invalidate checks without cancelling chat approvals or browser sign-ins. Check
transports are short-lived; no business tools execute during inspection.

## Connection cleanup

Connections belong to one runtime and close on transport failure/drop. Preserve
valid tool-error details as failed results while keeping the session usable.
Unix terminates owned process groups; Windows contains the server in a
kill-on-close Job Object before resuming it, so descendants are cleaned up too.
