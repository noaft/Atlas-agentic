use std::path::Path;

use crate::{
    agents::{AgentRegistry, LocalAgent, LocalAgentDefinition},
    models::{
        list_models_from_definition_with_settings, provider_from_definition_with_settings,
        ModelInfo, ModelProvider,
    },
    settings::{EffectiveSettings, SettingsStore},
    tools::{CreateFileTool, CurrentTimeTool, ListDirTool, PwdTool, ReadFileTool, ToolRegistry},
    Runtime,
};

#[cfg(unix)]
use crate::tools::ExecCommandTool;

pub fn build_runtime(workspace_root: &Path, agent_file: &Path) -> Result<Runtime, String> {
    let settings = SettingsStore::from_user_config()?
        .load()?
        .resolve_from_env()?;
    build_runtime_with_settings(workspace_root, agent_file, &settings)
}

pub fn build_runtime_with_settings(
    workspace_root: &Path,
    agent_file: &Path,
    settings: &EffectiveSettings,
) -> Result<Runtime, String> {
    let definition = LocalAgentDefinition::from_file(agent_file)?;
    let provider: Box<dyn ModelProvider> =
        provider_from_definition_with_settings(&definition, settings)?;
    let tools = build_default_tools(workspace_root);

    Ok(
        Runtime::new(AgentRegistry::new(vec![Box::new(LocalAgent::new(
            definition, provider,
        ))]))
        .with_tools(tools),
    )
}

fn build_default_tools(workspace_root: &Path) -> ToolRegistry {
    let tools = ToolRegistry::new(vec![
        Box::new(PwdTool::new(workspace_root)),
        Box::new(ListDirTool::new(workspace_root)),
        Box::new(ReadFileTool::new(workspace_root)),
        Box::new(CreateFileTool::new(workspace_root)),
        Box::new(CurrentTimeTool::new()),
    ]);
    #[cfg(unix)]
    let tools = {
        let mut tools = tools;
        tools.register(Box::new(ExecCommandTool::new(workspace_root)));
        tools
    };

    tools
}

pub fn list_available_models_with_settings(
    agent_file: &Path,
    settings: &EffectiveSettings,
) -> Result<Vec<ModelInfo>, String> {
    let definition = LocalAgentDefinition::from_file(agent_file)?;
    list_models_from_definition_with_settings(&definition, settings)
}

pub fn load_dotenv() {
    if std::env::var_os("ATLAS_SKIP_DOTENV").is_none() {
        let _ = dotenvy::dotenv();
    }
}

#[cfg(test)]
mod tests {
    use super::build_default_tools;

    #[test]
    fn default_tools_include_the_machine_clock() {
        let workspace = tempfile::tempdir().unwrap();

        let names = build_default_tools(workspace.path())
            .definitions()
            .into_iter()
            .map(|tool| tool.name)
            .collect::<Vec<_>>();

        assert!(names.iter().any(|name| name == "current_time"), "{names:?}");
    }
}
