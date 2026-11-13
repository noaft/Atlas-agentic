use super::{Tool, ToolDefinition, ToolRegistry, ToolResult};
use crate::{
    agent_catalog::{AgentCatalog, AgentDefinition},
    permissions::ToolRisk,
};
use serde::Deserialize;

pub fn register_agent_catalog_tools(registry: &mut ToolRegistry, catalog: AgentCatalog) {
    registry.register(crate::executions::worker::tool(CreateAgentTool(
        catalog.clone(),
    )));
    registry.register(crate::executions::worker::tool(ListAgentsTool(catalog)));
}
struct CreateAgentTool(AgentCatalog);
struct ListAgentsTool(AgentCatalog);
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CreateArgs {
    scope: String,
    definition: AgentDefinition,
}
impl CreateArgs {
    fn parse(args: &serde_json::Value) -> Result<Self, String> {
        let args: Self =
            serde_json::from_value(args.clone()).map_err(|_| "invalid create_agent arguments")?;
        if !matches!(args.scope.as_str(), "personal" | "project") {
            return Err("scope must be personal or project".into());
        }
        args.definition.validate()?;
        Ok(args)
    }
}
impl Tool for CreateAgentTool {
    fn name(&self) -> &str {
        "create_agent"
    }
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: self.name().into(),
            description: "Persist a reusable agent definition only when explicitly requested. Never overwrites. Each run receives caller-selected tools and fresh context; definitions do not retain conversation memory.".into(),
            input_schema: serde_json::json!({"type":"object","additionalProperties":false,"required":["scope","definition"],"properties":{
                "scope":{"type":"string","enum":["personal","project"]},
                "definition":{"type":"object","additionalProperties":false,"required":["name","display_name","description","system_prompt"],"properties":{
                    "name":{"type":"string","pattern":"^[a-z0-9-]{1,64}$"},
                    "display_name":{"type":"string"},"description":{"type":"string","maxLength":1024},"system_prompt":{"type":"string","maxLength":65536},
                    "provider":{"type":"string","description":"Optional provider override, requires model."},"model":{"type":"string","description":"Optional model override, requires provider."},
                    "skills":{"type":"array","items":{"type":"string"},"maxItems":128},
                    "max_tool_calls":{"type":"integer","minimum":1,"maximum":1000,"default":40},"timeout_seconds":{"type":"integer","minimum":1,"maximum":3600,"default":600}
                }}
            }}),
        }
    }
    fn risk(&self, args: &serde_json::Value) -> Result<ToolRisk, String> {
        Ok(match CreateArgs::parse(args)?.scope.as_str() {
            "personal" => ToolRisk::ModifyOutsideWorkspace,
            _ => ToolRisk::ModifyWorkspace,
        })
    }
    fn summarize(&self, args: &serde_json::Value) -> String {
        super::summarize_arguments(
            &serde_json::json!({"scope":args.get("scope"),"name":args.pointer("/definition/name")}),
        )
    }
    fn execute(&self, args: &serde_json::Value) -> Result<ToolResult, String> {
        let args = CreateArgs::parse(args)?;
        let info = self.0.create(&args.scope, args.definition)?;
        Ok(ToolResult::text(
            serde_json::to_string(&serde_json::json!({"id":info.id,"display_name":info.definition.display_name,"created":true})).map_err(|_| "failed to encode agent definition")?,
        ))
    }
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ListArgs {}
fn validate_list(args: &serde_json::Value) -> Result<(), String> {
    serde_json::from_value::<ListArgs>(args.clone())
        .map(|_| ())
        .map_err(|_| "list_agents expects an empty object".into())
}
impl Tool for ListAgentsTool {
    fn name(&self) -> &str {
        "list_agents"
    }
    fn definition(&self) -> ToolDefinition {
        ToolDefinition { name: self.name().into(), description: "List reusable personal and project agent definitions, with scoped IDs. Tool access is chosen separately for every run.".into(), input_schema: serde_json::json!({"type":"object","additionalProperties":false,"properties":{}}) }
    }
    fn risk(&self, args: &serde_json::Value) -> Result<ToolRisk, String> {
        validate_list(args)?;
        Ok(ToolRisk::ReadOnly)
    }
    fn execute(&self, args: &serde_json::Value) -> Result<ToolResult, String> {
        validate_list(args)?;
        let infos = self.0.list()?;
        let total = infos.len();
        let mut data = Vec::new();
        let mut encoded_size = 0;
        for info in infos {
            let definition = info.definition;
            let metadata = serde_json::json!({
                "id":info.id, "name":definition.name, "display_name":definition.display_name,
                "description":definition.description, "provider":definition.provider,
                "model":definition.model, "skills":definition.skills,
                "max_tool_calls":definition.max_tool_calls, "timeout_seconds":definition.timeout_seconds
            });
            let size = serde_json::to_vec(&metadata)
                .map_err(|_| "failed to encode agent metadata")?
                .len();
            // Reserve room for separators and the fixed envelope, including its counts.
            if encoded_size + size + data.len() + 256 > 64 * 1024 {
                break;
            }
            encoded_size += size;
            data.push(metadata);
        }
        let truncated = data.len() < total;
        let envelope = serde_json::json!({"data":data,"total":total,"truncated":truncated});
        Ok(ToolResult::text(
            serde_json::to_string(&envelope).map_err(|_| "failed to encode agent catalog")?,
        ))
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn listing_is_bounded_metadata_without_prompts() {
        let dir = tempfile::tempdir().unwrap();
        let catalog = AgentCatalog::new(dir.path().join("p"), dir.path().join("w"));
        for i in 0..64 {
            let definition = serde_json::from_value(serde_json::json!({
                "name":format!("agent-{i}"), "display_name":"Display".repeat(140),
                "description":"Description".repeat(90), "system_prompt":"PRIVATE_PROMPT".repeat(4000)
            })).unwrap();
            catalog.create("personal", definition).unwrap();
        }
        let tool = ListAgentsTool(catalog);
        let output = tool.execute(&serde_json::json!({})).unwrap().message;
        assert!(output.len() <= 65536);
        assert!(!output.contains("PRIVATE_PROMPT"));
        assert!(!output.contains("system_prompt"));
        let value: serde_json::Value = serde_json::from_str(&output).unwrap();
        assert_eq!(value["truncated"], true);
        assert_eq!(value["total"], 64);
        assert!(!value["data"].as_array().unwrap().is_empty());
        assert_eq!(value["data"][0]["id"], "personal:agent-0");
    }

    #[test]
    fn rejects_oversized_serialization_before_authorization() {
        let dir = tempfile::tempdir().unwrap();
        let mut registry = ToolRegistry::empty();
        register_agent_catalog_tools(
            &mut registry,
            AgentCatalog::new(dir.path().join("p"), dir.path().join("w")),
        );
        let args = serde_json::json!({"scope":"project", "definition":{"name":"helper", "display_name":"Helper", "description":"Help", "system_prompt":"\u{1}".repeat(65536)}});
        assert!(registry.authorization("create_agent", &args).is_err());
    }
    #[test]
    fn validates_before_authorizing_and_applies_scope_risk() {
        let dir = tempfile::tempdir().unwrap();
        let mut registry = ToolRegistry::empty();
        register_agent_catalog_tools(
            &mut registry,
            AgentCatalog::new(dir.path().join("p"), dir.path().join("w")),
        );
        let mut args = serde_json::json!({"scope":"project","definition":{"name":"helper","display_name":"Helper","description":"Help","system_prompt":"Do the assigned task"}});
        assert_eq!(
            registry
                .authorization("create_agent", &args)
                .unwrap()
                .risk(),
            ToolRisk::ModifyWorkspace
        );
        args["scope"] = "personal".into();
        assert_eq!(
            registry
                .authorization("create_agent", &args)
                .unwrap()
                .risk(),
            ToolRisk::ModifyOutsideWorkspace
        );
        args["definition"]["tools"] = serde_json::json!([]);
        assert!(registry.authorization("create_agent", &args).is_err());
        assert!(registry
            .authorization("list_agents", &serde_json::json!({"extra":true}))
            .is_err());
        assert_eq!(
            registry
                .authorization("list_agents", &serde_json::json!({}))
                .unwrap()
                .risk(),
            ToolRisk::ReadOnly
        );
        assert!(!dir.path().join("p").exists());
    }
}
