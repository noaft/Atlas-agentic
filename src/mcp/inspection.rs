//! Explicit Settings inspection, through the same authorization path as chat discovery.
use super::{register_tools_with_cancel, McpConfig};
use crate::{
    agent_output::AgentOutput,
    permissions::{
        requires_confirmation, AuthorizationDecision, AuthorizationRequest, PermissionMode,
        ToolAuthorizer,
    },
    tools::ToolRegistry,
};
use serde::{Deserialize, Serialize};
use std::{
    path::Path,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InspectedTool {
    pub name: String,
    pub description: String,
    pub input_schema: serde_json::Value,
}
#[derive(Deserialize)]
struct Catalog {
    tools: Vec<InspectedTool>,
}
fn check_cancelled(cancelled: &AtomicBool) -> Result<(), String> {
    if cancelled.load(Ordering::SeqCst) {
        Err("MCP tool check cancelled".into())
    } else {
        Ok(())
    }
}
/// Initializes and lists only; dropping the local registry closes all owned transports.
pub fn inspect_server_tools(
    mut config: McpConfig,
    server: &str,
    workspace: &Path,
    mode: PermissionMode,
    authorizer: &dyn ToolAuthorizer,
    cancelled: &Arc<AtomicBool>,
) -> Result<Vec<InspectedTool>, String> {
    check_cancelled(cancelled)?;
    let selected = config
        .servers
        .get_mut(server)
        .ok_or("Unknown configured MCP server")?;
    selected.timeout_ms = selected.timeout_ms.min(30_000);
    // No other configured connection can be executed by this operation.
    config.servers.retain(|name, _| name == server);
    let mut registry = ToolRegistry::empty();
    register_tools_with_cancel(&mut registry, config, workspace, Some(cancelled.clone()));
    let args = serde_json::json!({"server":server});
    let authorization = registry.authorization("mcp_list_tools", &args)?;
    if requires_confirmation(mode, authorization.risk()) {
        let request = AuthorizationRequest::new(
            "mcp_list_tools",
            authorization.arguments().into(),
            authorization.risk(),
        );
        if authorizer.authorize(&request)? == AuthorizationDecision::Deny {
            return Err("MCP tool check denied".into());
        }
    }
    check_cancelled(cancelled)?;
    let result = registry.execute_authorized(
        &AgentOutput::ToolCall {
            tool: "mcp_list_tools".into(),
            args,
            message: None,
        },
        &authorization,
    )?;
    check_cancelled(cancelled)?;
    let catalog: Catalog =
        serde_json::from_str(&result.message).map_err(|_| "Invalid MCP inspection result")?;
    Ok(catalog.tools)
}
