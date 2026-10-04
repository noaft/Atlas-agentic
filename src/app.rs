use std::path::Path;

use crate::{
    agents::{AgentRegistry, LocalAgent, LocalAgentDefinition},
    models::provider_from_definition,
    tools::{CreateFileTool, ListDirTool, PwdTool, ReadFileTool, ToolRegistry},
    Runtime,
};

#[cfg(unix)]
use crate::tools::ExecCommandTool;

pub fn build_runtime(workspace_root: &Path, agent_file: &Path) -> Result<Runtime, String> {
    let definition = LocalAgentDefinition::from_file(agent_file)?;
    let provider = provider_from_definition(&definition)?;
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

pub fn load_dotenv() {
    if std::env::var_os("ATLAS_SKIP_DOTENV").is_none() {
        let _ = dotenvy::dotenv();
    }
}
