use std::path::Path;

use crate::{
    agents::{AgentRegistry, LocalAgent, LocalAgentDefinition},
    models::{
        list_models_from_definition_with_settings, provider_from_definition_with_settings,
        ModelInfo, ModelProvider,
    },
    settings::{AtlasSettings, EffectiveSettings, SettingsStore},
    tools::{
        system_computer_controller, CaptureScreenTool, CopyFileTool, CreateDirectoryTool,
        CreateFileTool, CurrentTimeTool, DeleteFileTool, GetUiStateTool, KeyPressTool, ListDirTool,
        MouseClickTool, MouseMoveTool, MouseScrollTool, MoveFileTool, OpenApplicationTool,
        OpenFileTool, PwdTool, ReadFileTool, RipgrepTool, SearchFilesTool, ToolRegistry,
        TypeTextTool,
    },
    Runtime,
};

#[cfg(any(unix, windows))]
use crate::tools::ExecCommandTool;

/// Settings inspection stays available when an agent file is missing; runtime construction
/// still reports invalid definitions before provider setup. Valid definitions supply defaults.
pub fn resolve_settings_for_agent(
    saved: &AtlasSettings,
    agent_file: &Path,
) -> Result<EffectiveSettings, String> {
    match LocalAgentDefinition::from_file(agent_file) {
        Ok(definition) => {
            saved.resolve_for(&definition.model_provider, &definition.model, |name| {
                std::env::var_os(name)
            })
        }
        Err(_) => saved.resolve_from_env(),
    }
}

pub fn build_runtime(workspace_root: &Path, agent_file: &Path) -> Result<Runtime, String> {
    let definition = LocalAgentDefinition::from_file(agent_file)?;
    let settings = SettingsStore::from_user_config()?.load()?.resolve_for(
        &definition.model_provider,
        &definition.model,
        |name| std::env::var_os(name),
    )?;
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
    let mut tools = build_default_tools(workspace_root);
    crate::mcp::register_tools(
        &mut tools,
        crate::mcp::McpConfig::from_environment()?,
        workspace_root,
    );

    Ok(
        Runtime::new(AgentRegistry::new(vec![Box::new(LocalAgent::new(
            definition, provider,
        ))]))
        .with_tools(tools),
    )
}

fn build_default_tools(workspace_root: &Path) -> ToolRegistry {
    let computer = system_computer_controller();
    let tools = ToolRegistry::new(vec![
        Box::new(PwdTool::new(workspace_root)),
        Box::new(ListDirTool::new(workspace_root)),
        Box::new(ReadFileTool::new(workspace_root)),
        Box::new(CreateFileTool::new(workspace_root)),
        Box::new(CreateDirectoryTool::new(workspace_root)),
        Box::new(CopyFileTool::new(workspace_root)),
        Box::new(MoveFileTool::new(workspace_root)),
        Box::new(DeleteFileTool::new(workspace_root)),
        Box::new(RipgrepTool::new(workspace_root)),
        Box::new(SearchFilesTool::new(workspace_root)),
        Box::new(OpenFileTool::new(workspace_root)),
        Box::new(OpenApplicationTool::new()),
        Box::new(CurrentTimeTool::new()),
        Box::new(MouseMoveTool::new(computer.clone())),
        Box::new(MouseClickTool::new(computer.clone())),
        Box::new(MouseScrollTool::new(computer.clone())),
        Box::new(TypeTextTool::new(computer.clone())),
        Box::new(KeyPressTool::new(computer.clone())),
        Box::new(GetUiStateTool::new(computer.clone())),
        Box::new(CaptureScreenTool::new(computer)),
    ]);
    #[cfg(any(unix, windows))]
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
    fn default_tools_include_the_workspace_file_family_and_machine_clock() {
        let workspace = tempfile::tempdir().unwrap();

        let mut names = build_default_tools(workspace.path())
            .definitions()
            .into_iter()
            .map(|tool| tool.name)
            .collect::<Vec<_>>();
        names.sort();

        let mut expected = vec![
            "capture_screen",
            "copy_file",
            "create_directory",
            "create_file",
            "current_time",
            "delete_file",
            "get_ui_state",
            "key_press",
            "ls",
            "move_file",
            "mouse_click",
            "mouse_move",
            "mouse_scroll",
            "open_application",
            "open_file",
            "pwd",
            "read_file",
            "ripgrep",
            "search_files",
            "type_text",
        ];
        #[cfg(any(unix, windows))]
        expected.push("exec_command");
        expected.sort();

        assert_eq!(names, expected);
    }
}
