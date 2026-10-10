use std::path::Path;

use crate::{
    agents::{AgentRegistry, LocalAgent, LocalAgentDefinition},
    models::{
        list_models_from_definition_with_settings, provider_from_definition_with_settings,
        ModelInfo, ModelProvider,
    },
    settings::{EffectiveSettings, SettingsStore},
    tools::{CreateFileTool, ListDirTool, PwdTool, ReadFileTool, ToolRegistry},
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
    let tools = ToolRegistry::new(vec![
        Box::new(PwdTool::new(workspace_root)),
        Box::new(ListDirTool::new(workspace_root)),
        Box::new(ReadFileTool::new(workspace_root)),
        Box::new(CreateFileTool::new(workspace_root)),
    ]);
    #[cfg(unix)]
    let tools = {
        let mut tools = tools;
        tools.register(Box::new(ExecCommandTool::new(workspace_root)));
        tools
    };

    Ok(
        Runtime::new(AgentRegistry::new(vec![Box::new(LocalAgent::new(
            definition, provider,
        ))]))
        .with_tools(tools),
    )
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
