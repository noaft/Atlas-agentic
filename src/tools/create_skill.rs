use std::collections::BTreeMap;

use serde::Deserialize;

use crate::{
    permissions::ToolRisk,
    skills::{SkillDraftInput, SkillScope, SkillStore},
};

use super::{summarize_arguments, Tool, ToolDefinition, ToolResult};

/// Saves explicitly requested skill packages through the normal runtime authorization path.
pub struct CreateSkillTool {
    store: SkillStore,
}

impl CreateSkillTool {
    pub fn new(store: SkillStore) -> Self {
        Self { store }
    }
}

// Explicit fields retain deny_unknown_fields validation without serde flatten ambiguity.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CreateSkillArgs {
    scope: SkillScope,
    name: String,
    title: Option<String>,
    description: String,
    instructions: String,
    files: Option<BTreeMap<String, String>>,
}

impl CreateSkillArgs {
    fn parse(args: &serde_json::Value) -> Result<(SkillScope, SkillDraftInput), String> {
        let args: Self = serde_json::from_value(args.clone())
            .map_err(|error| format!("invalid create_skill args: {error}"))?;
        let input = SkillDraftInput {
            name: args.name,
            title: args.title,
            description: args.description,
            instructions: args.instructions,
            files: args.files,
        };
        input.validate()?;
        Ok((args.scope, input))
    }
}

impl Tool for CreateSkillTool {
    fn name(&self) -> &str {
        "create_skill"
    }

    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: self.name().into(),
            description: "Save a new reusable skill only when the user explicitly asks to save or create a skill. Never save skills proactively. Project skills live in the starting workspace; personal skills live in the user's shared skills folder. Uses normal approval, never overwrites existing skills, and never executes scripts. Returns the skill ID; the skill becomes available on the next request.".into(),
            input_schema: serde_json::json!({
                "type":"object",
                "additionalProperties":false,
                "required":["scope","name","description","instructions"],
                "properties": {
                    "scope":{"type":"string","enum":["personal","project"]},
                    "name":{"type":"string","description":"Folder name: lowercase letters, digits and single hyphens; 1–64 bytes."},
                    "title":{"type":"string","description":"Optional display title."},
                    "description":{"type":"string","description":"Short summary of when to use the skill."},
                    "instructions":{"type":"string","description":"Nonempty Markdown instructions, without YAML frontmatter."},
                    "files":{"type":"object","additionalProperties":{"type":"string"},"description":"Optional supporting UTF-8 files keyed by safe relative path. Cannot replace SKILL.md."}
                }
            }),
        }
    }

    fn risk(&self, args: &serde_json::Value) -> Result<ToolRisk, String> {
        let (scope, _) = CreateSkillArgs::parse(args)?;
        Ok(match scope {
            SkillScope::Project => ToolRisk::ModifyWorkspace,
            SkillScope::Personal => ToolRisk::ModifyOutsideWorkspace,
        })
    }

    fn summarize(&self, args: &serde_json::Value) -> String {
        let mut summary = serde_json::json!({});
        for field in ["scope", "name", "title", "description"] {
            if let Some(value) = args.get(field) {
                summary[field] = value.clone();
            }
        }
        if let Some(files) = args.get("files").and_then(serde_json::Value::as_object) {
            summary["files"] = serde_json::json!(files.keys().collect::<Vec<_>>());
        }
        summarize_arguments(&summary)
    }

    fn execute(&self, args: &serde_json::Value) -> Result<ToolResult, String> {
        let (scope, input) = CreateSkillArgs::parse(args)?;
        let info = self.store.create(input, scope)?;
        let result = serde_json::to_string(&info)
            .map_err(|_| "failed to encode created skill metadata".to_string())?;
        Ok(ToolResult::text(result))
    }
}
