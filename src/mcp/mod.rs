//! MCP connections exposed through Atlas's normal runtime authorization boundary.
mod config;
mod http;
pub mod oauth;
mod stdio;
#[cfg(windows)]
mod windows;

pub use config::{McpConfig, OAuthConfig, ServerConfig};

use std::{
    cell::RefCell,
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
    rc::Rc,
};

use serde::Deserialize;
use serde_json::{json, Value};

use crate::{
    permissions::ToolRisk,
    tools::{Tool, ToolDefinition, ToolRegistry, ToolResult},
};

enum Client {
    Stdio(stdio::Client),
    Http(http::Client),
}

impl Client {
    fn connect(config: &ServerConfig, workspace: &Path) -> Result<Self, String> {
        if config.url.is_some() {
            http::Client::connect(config).map(Self::Http)
        } else {
            stdio::Client::connect(config, workspace).map(Self::Stdio)
        }
    }

    fn request(&mut self, method: &str, params: Value) -> Result<Value, String> {
        match self {
            Self::Stdio(client) => client.request(method, params),
            Self::Http(client) => client.request(method, params),
        }
    }
}

struct Connections {
    config: McpConfig,
    workspace: PathBuf,
    clients: BTreeMap<String, Client>,
}

impl Connections {
    fn validate_server(&self, server: &str) -> Result<(), String> {
        if self.config.servers.contains_key(server) {
            Ok(())
        } else {
            Err("unknown configured MCP server".into())
        }
    }

    fn run(
        &mut self,
        server: &str,
        operation: impl FnOnce(&mut Client) -> Result<Value, String>,
    ) -> Result<Value, String> {
        self.validate_server(server)?;
        if !self.clients.contains_key(server) {
            let client = Client::connect(&self.config.servers[server], &self.workspace)?;
            self.clients.insert(server.into(), client);
        }
        let result = operation(
            self.clients
                .get_mut(server)
                .ok_or("MCP connection unavailable")?,
        );
        if result.is_err() {
            // Never retry a call: the server may have performed the action before failing.
            self.clients.remove(server);
        }
        result
    }
}

#[derive(Clone, Copy)]
enum Operation {
    Servers,
    List,
    Call,
}

struct McpTool {
    operation: Operation,
    connections: Rc<RefCell<Connections>>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct EmptyArgs {}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ListArgs {
    server: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CallArgs {
    server: String,
    tool: String,
    arguments: serde_json::Map<String, Value>,
}

fn parse<T: serde::de::DeserializeOwned>(args: &Value) -> Result<T, String> {
    serde_json::from_value(args.clone()).map_err(|_| "invalid MCP tool arguments".into())
}

impl Tool for McpTool {
    fn name(&self) -> &str {
        match self.operation {
            Operation::Servers => "mcp_list_servers",
            Operation::List => "mcp_list_tools",
            Operation::Call => "mcp_call_tool",
        }
    }

    fn definition(&self) -> ToolDefinition {
        let servers: Vec<String> = self
            .connections
            .borrow()
            .config
            .servers
            .keys()
            .cloned()
            .collect();
        let (description, properties, required) = match self.operation {
            Operation::Servers => (
                "List configured MCP server names without launching processes. Use mcp_list_tools to discover a server's tools.",
                json!({}), json!([]),
            ),
            Operation::List => (
                "Connect to a configured MCP server and list its tools with input schemas. May launch a process or make HTTP requests. Use mcp_call_tool with the returned tool name and arguments matching its inputSchema.",
                json!({"server":{"type":"string", "enum":servers}}), json!(["server"]),
            ),
            Operation::Call => (
                "Call a tool on a configured MCP server. Discover its name and inputSchema with mcp_list_tools first. External tools may read sensitive data or change the system; server annotations do not grant permission. Failed or timed-out calls may have changed external state: do not automatically retry them.",
                json!({"server":{"type":"string", "enum":servers},"tool":{"type":"string","minLength":1},"arguments":{"type":"object"}}),
                json!(["server","tool","arguments"]),
            ),
        };
        ToolDefinition {
            name: self.name().into(),
            description: description.into(),
            input_schema: json!({"type":"object", "properties":properties, "required":required, "additionalProperties":false}),
        }
    }

    fn risk(&self, args: &Value) -> Result<ToolRisk, String> {
        match self.operation {
            Operation::Servers => {
                let _: EmptyArgs = parse(args)?;
                Ok(ToolRisk::ReadOnly)
            }
            Operation::List => {
                let args: ListArgs = parse(args)?;
                self.connections.borrow().validate_server(&args.server)?;
                Ok(
                    if self.connections.borrow().config.servers[&args.server]
                        .url
                        .is_some()
                    {
                        ToolRisk::ExternalTool
                    } else {
                        ToolRisk::Launch
                    },
                )
            }
            Operation::Call => {
                let args: CallArgs = parse(args)?;
                if args.tool.trim().is_empty() || args.tool.len() > 128 {
                    return Err("MCP tool name must contain 1–128 bytes".into());
                }
                self.connections.borrow().validate_server(&args.server)?;
                Ok(ToolRisk::ExternalTool)
            }
        }
    }

    fn execute(&self, args: &Value) -> Result<ToolResult, String> {
        self.risk(args)?;
        let mut connections = self.connections.borrow_mut();
        let result = match self.operation {
            Operation::Servers => {
                json!({"servers":connections.config.servers.keys().collect::<Vec<_>>()})
            }
            Operation::List => {
                let args: ListArgs = parse(args)?;
                connections.run(&args.server, list_tools)?
            }
            Operation::Call => {
                let args: CallArgs = parse(args)?;
                let result = connections.run(&args.server, |client| {
                    client.request(
                        "tools/call",
                        json!({"name":args.tool,"arguments":args.arguments}),
                    )
                })?;
                text_result(result)?
            }
        };
        Ok(ToolResult::text(
            serde_json::to_string(&result).map_err(|_| "could not encode MCP tool result")?,
        ))
    }
}

fn list_tools(client: &mut Client) -> Result<Value, String> {
    let mut tools = Vec::new();
    let mut names = BTreeSet::new();
    let mut cursors = BTreeSet::new();
    let mut params = json!({});
    let mut total_bytes = 0;
    for _ in 0..16 {
        let page = client.request("tools/list", params)?;
        total_bytes += serde_json::to_vec(&page)
            .map_err(|_| "invalid MCP tool catalog")?
            .len();
        if total_bytes > 1024 * 1024 {
            return Err("MCP tool catalog exceeds the 1 MiB limit".into());
        }
        for tool in page["tools"].as_array().ok_or("invalid MCP tool list")? {
            let name = tool["name"]
                .as_str()
                .filter(|name| !name.is_empty() && name.len() <= 128)
                .ok_or("invalid MCP tool name")?;
            if !names.insert(name.to_string()) || tools.len() >= 256 {
                return Err("MCP tool catalog has duplicate names or exceeds 256 tools".into());
            }
            if tool["inputSchema"]["type"] != "object" || !tool["inputSchema"].is_object() {
                return Err("invalid MCP tool input schema".into());
            }
            let description = match tool.get("description") {
                Some(value) => value.as_str().ok_or("invalid MCP tool description")?,
                None => "",
            };
            // Only expose metadata needed for tool use, excluding icons and arbitrary binary data.
            tools.push(
                json!({"name":name,"description":description,"inputSchema":tool["inputSchema"]}),
            );
        }
        match page.get("nextCursor") {
            None => return Ok(json!({"tools":tools})),
            Some(value) => {
                let cursor = value
                    .as_str()
                    .filter(|cursor| !cursor.is_empty())
                    .ok_or("invalid MCP pagination cursor")?;
                if !cursors.insert(cursor.to_string()) {
                    return Err("MCP server repeated a pagination cursor".into());
                }
                params = json!({"cursor":cursor});
            }
        }
    }
    Err("MCP tool catalog exceeds 16 pages".into())
}

fn text_result(result: Value) -> Result<Value, String> {
    if let Some(value) = result.get("isError") {
        if !value.is_boolean() {
            return Err("invalid MCP tool error flag".into());
        }
    }
    let mut content = Vec::new();
    for item in result["content"]
        .as_array()
        .ok_or("invalid MCP tool content")?
    {
        if item["type"] != "text" {
            return Err(
                "MCP returned unsupported non-text content; action may have completed".into(),
            );
        }
        let text = item["text"].as_str().ok_or("invalid MCP text content")?;
        content.push(json!({"type":"text", "text":text}));
    }
    let failed = result.get("isError") == Some(&Value::Bool(true));
    let mut output = json!({"content":content, "isError":failed});
    if let Some(structured) = result.get("structuredContent") {
        if !structured.is_object() {
            return Err("invalid MCP structured content".into());
        }
        output["structuredContent"] = structured.clone();
    }
    if failed {
        // Application errors are normal tool output. Keep corrective details available to
        // the agent, without promoting a failed tool call to a successful runtime result.
        return Err(format!("MCP tool reported an error: {output}"));
    }
    Ok(output)
}

/// Register gateways without spawning servers; all side effects stay in tool execution.
pub fn register_tools(registry: &mut ToolRegistry, config: McpConfig, workspace: &Path) {
    if config.servers.is_empty() {
        return;
    }
    let connections = Rc::new(RefCell::new(Connections {
        config,
        workspace: workspace.to_path_buf(),
        clients: BTreeMap::new(),
    }));
    for operation in [Operation::Servers, Operation::List, Operation::Call] {
        registry.register(Box::new(McpTool {
            operation,
            connections: Rc::clone(&connections),
        }));
    }
}
