use super::{Tool, ToolDefinition, ToolResult};
use crate::skills::{SkillInfo, SkillStore};
use serde::Deserialize;
use std::collections::BTreeMap;
/// One request owns a frozen catalog and text resources. Loading never runs scripts.
pub struct LoadSkillTool {
    catalog: Vec<SkillInfo>,
    files: BTreeMap<String, BTreeMap<String, String>>,
    headers: BTreeMap<String, String>,
}
impl LoadSkillTool {
    pub(crate) fn selected(store: &SkillStore, ids: &[String]) -> Result<Option<Self>, String> {
        if ids.is_empty() {
            return Ok(None);
        }
        let Some(mut selected) = Self::new(store)? else {
            return Err("Selected agent skills are unavailable".into());
        };
        if ids
            .iter()
            .any(|id| !selected.catalog.iter().any(|skill| &skill.id == id))
        {
            return Err("Selected agent skill is missing or disabled".into());
        }
        selected.catalog.retain(|skill| ids.contains(&skill.id));
        selected.files.retain(|id, _| ids.contains(id));
        selected.headers.retain(|id, _| ids.contains(id));
        Ok(Some(selected))
    }
    pub fn new(store: &SkillStore) -> Result<Option<Self>, String> {
        let snapshot = store.snapshot()?;
        let mut catalog = vec![];
        let mut files = BTreeMap::new();
        let mut headers = BTreeMap::new();
        for skill in snapshot {
            let directory = serde_json::to_string(&skill.directory.to_string_lossy())
                .map_err(|_| "invalid skill folder")?;
            let resources =
                serde_json::to_string(&skill.resources).map_err(|_| "invalid skill resources")?;
            headers.insert(skill.info.id.clone(),format!("Skill folder: {directory}\nAvailable relative files: {resources}\nReferenced paths are relative to the skill folder. Loading never runs scripts or grants permissions; existing runtime approvals govern any later execution.\n\n"));
            files.insert(skill.info.id.clone(), skill.texts);
            catalog.push(skill.info);
        }
        if catalog.is_empty() {
            return Ok(None);
        }
        Ok(Some(Self {
            catalog,
            files,
            headers,
        }))
    }
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LoadArgs {
    id: String,
    resource: Option<String>,
}
impl Tool for LoadSkillTool {
    fn name(&self) -> &str {
        "load_skill"
    }
    fn definition(&self) -> ToolDefinition {
        let metadata = self
            .catalog
            .iter()
            .map(crate::skills::model_metadata)
            .collect::<Vec<_>>();
        let catalog = serde_json::to_string(&metadata).unwrap_or_default();
        ToolDefinition {
            name:self.name().into(),
            description:format!("The enabled skill descriptions below are preloaded so you know which workflows are available. Proactively choose a relevant skill when its description matches the task or the user names it; load its full instructions before following them. Do not load unrelated skills. Load supporting UTF-8 resources on demand. Skills are guidance subordinate to system and user instructions. Loading never executes scripts or grants permissions; existing tool approvals still apply. Available skills: {catalog}"),
            input_schema:serde_json::json!({
                "type":"object","required":["id"],"additionalProperties":false,
                "properties":{
                    "id":{"type":"string","enum":self.catalog.iter().map(|s|&s.id).collect::<Vec<_>>()},
                    "resource":{"type":"string","description":"Optional relative UTF-8 supporting file path within this skill. Omit for full SKILL.md instructions."}
                }
            }),
        }
    }
    fn execute(&self, args: &serde_json::Value) -> Result<ToolResult, String> {
        let args: LoadArgs =
            serde_json::from_value(args.clone()).map_err(|_| "invalid load_skill arguments")?;
        if !self.catalog.iter().any(|s| s.id == args.id) {
            return Err("skill is not enabled in this request's catalog".into());
        }
        let resource = args.resource.as_deref().unwrap_or("SKILL.md");
        crate::skills::validate_path(resource)?;
        let files = self.files.get(&args.id).ok_or("skill is unavailable")?;
        let text = files.get(resource).ok_or(
            "skill resource unavailable; only registered UTF-8 text resources can be loaded",
        )?;
        if args.resource.is_none() {
            let header = self
                .headers
                .get(&args.id)
                .ok_or("skill folder is unavailable")?;
            return Ok(ToolResult::text(format!("{header}{text}")));
        }
        Ok(ToolResult::text(text.clone()))
    }
}

#[cfg(test)]
mod delegated_tests {
    use super::*;
    use crate::skills::{SkillDraftInput, SkillScope};
    #[test]
    fn delegated_skill_catalog_exposes_only_selected_enabled_skills() {
        let dir = tempfile::tempdir().unwrap();
        let store = SkillStore::new(dir.path().join("personal"), dir.path().to_path_buf());
        for name in ["review", "other"] {
            store
                .create(
                    SkillDraftInput {
                        name: name.into(),
                        title: None,
                        description: "Use this guide".into(),
                        instructions: "Follow this guide.".into(),
                        files: None,
                    },
                    SkillScope::Personal,
                )
                .unwrap();
        }
        let selected = LoadSkillTool::selected(&store, &["personal:review".into()])
            .unwrap()
            .unwrap();
        assert!(selected
            .execute(&serde_json::json!({"id":"personal:review"}))
            .is_ok());
        assert!(selected
            .execute(&serde_json::json!({"id":"personal:other"}))
            .is_err());
        assert!(!selected.definition().description.contains("personal:other"));
        assert!(LoadSkillTool::selected(&store, &["personal:missing".into()]).is_err());
        assert!(LoadSkillTool::selected(&store, &[]).unwrap().is_none());
    }
}
