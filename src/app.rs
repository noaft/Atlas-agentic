use std::path::Path;

use crate::{
    agents::{AgentRegistry, LocalAgent, LocalAgentDefinition},
    models::{
        list_models_from_definition_with_settings, provider_from_definition_with_settings,
        ModelInfo, ModelProvider,
    },
    settings::{AtlasSettings, EffectiveSettings, SettingsStore},
    tools::{
        system_computer_controller, ApplyPatchTool, CaptureScreenTool, CopyFileTool,
        CreateDirectoryTool, CreateFileTool, CurrentTimeTool, DeleteFileTool, GetUiStateTool,
        KeyPressTool, ListDirTool, MouseClickTool, MouseMoveTool, MouseScrollTool, MoveFileTool,
        OpenApplicationTool, OpenFileTool, PwdTool, ReadFileTool, RipgrepTool, SearchFilesTool,
        ToolRegistry, TypeTextTool,
    },
    Runtime,
};

use crate::process_limits::ProcessLimits;

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
    let store = SettingsStore::from_user_config()?;
    let runtime = build_runtime_from_definition(workspace_root, definition, settings, &store)?;
    let runtime = runtime.with_delegation(crate::delegation::Delegation::new(
        workspace_root,
        settings.clone(),
        store,
    )?);
    #[cfg(any(unix, windows))]
    let runtime = {
        let manager = crate::tools::ProcessManager::new();
        manager.set_limits(settings.process_limits)?;
        let scope = manager.create_scope()?;
        let mut runtime = runtime.with_request_process_scope(workspace_root, scope.clone(), false);
        runtime.owned_process_scope = Some(scope);
        runtime
    };
    Ok(runtime)
}

pub(crate) fn build_runtime_from_definition(
    workspace_root: &Path,
    definition: LocalAgentDefinition,
    settings: &EffectiveSettings,
    store: &SettingsStore,
) -> Result<Runtime, String> {
    let provider: Box<dyn ModelProvider> =
        provider_from_definition_with_settings(&definition, settings)?;
    let mut tools = build_default_tools(workspace_root, settings.process_limits)?;
    let skill_store = crate::skills::SkillStore::for_settings(store, workspace_root.to_path_buf())?;
    if let Ok(Some(tool)) = crate::tools::LoadSkillTool::new(&skill_store) {
        tools.register(crate::executions::worker::tool(tool));
    }
    crate::tools::register_skill_import_tools(&mut tools, skill_store.clone());
    tools.register(crate::executions::worker::tool(
        crate::tools::CreateSkillTool::new(skill_store),
    ));
    crate::mcp::register_tools(
        &mut tools,
        crate::mcp::McpConfig::from_environment()?,
        workspace_root,
    );

    crate::tools::register_agent_catalog_tools(
        &mut tools,
        crate::agent_catalog::AgentCatalog::for_settings(store, workspace_root),
    );
    Ok(
        Runtime::new(AgentRegistry::new(vec![Box::new(LocalAgent::new(
            definition, provider,
        ))]))
        .with_tools(tools),
    )
}

fn build_default_tools(
    workspace_root: &Path,
    limits: ProcessLimits,
) -> Result<ToolRegistry, String> {
    limits.validate()?;
    let computer = system_computer_controller();
    let tools = ToolRegistry::new(vec![
        crate::executions::worker::tool(PwdTool::new(workspace_root)),
        crate::executions::worker::tool(ListDirTool::new(workspace_root)),
        crate::executions::worker::tool(ReadFileTool::new(workspace_root)),
        crate::executions::worker::tool(CreateFileTool::new(workspace_root)),
        crate::executions::worker::tool(ApplyPatchTool::new(workspace_root)),
        crate::executions::worker::tool(CreateDirectoryTool::new(workspace_root)),
        crate::executions::worker::tool(CopyFileTool::new(workspace_root)),
        crate::executions::worker::tool(MoveFileTool::new(workspace_root)),
        crate::executions::worker::tool(DeleteFileTool::new(workspace_root)),
        crate::executions::worker::tool(RipgrepTool::new(workspace_root)),
        crate::executions::worker::tool(SearchFilesTool::new(workspace_root)),
        crate::executions::worker::tool(OpenFileTool::new(workspace_root)),
        crate::executions::worker::tool(OpenApplicationTool::new()),
        crate::executions::worker::tool(CurrentTimeTool::new()),
        crate::executions::worker::tool(MouseMoveTool::new(computer.clone())),
        crate::executions::worker::tool(MouseClickTool::new(computer.clone())),
        crate::executions::worker::tool(MouseScrollTool::new(computer.clone())),
        crate::executions::worker::tool(TypeTextTool::new(computer.clone())),
        crate::executions::worker::tool(KeyPressTool::new(computer.clone())),
        crate::executions::worker::tool(GetUiStateTool::new(computer.clone())),
        crate::executions::worker::tool(CaptureScreenTool::new(computer)),
    ]);
    #[cfg(any(unix, windows))]
    let tools = {
        let mut tools = tools;
        let exec =
            crate::tools::ExecCommandTool::new(workspace_root).with_process_limits(limits)?;
        for tool in exec.session_tools() {
            tools.register(tool);
        }
        tools.register(crate::executions::worker::tool(exec));
        tools
    };

    Ok(tools)
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

        let mut names = build_default_tools(
            workspace.path(),
            crate::process_limits::ProcessLimits::default(),
        )
        .unwrap()
        .definitions()
        .into_iter()
        .map(|tool| tool.name)
        .collect::<Vec<_>>();
        names.sort();

        let mut expected = vec![
            "apply_patch",
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
        expected.extend([
            "exec_command",
            "list_processes",
            "read_process",
            "stop_process",
        ]);
        expected.sort();

        assert_eq!(names, expected);
    }
    #[test]
    fn runtime_exposes_creation_without_requiring_a_skill_catalog() {
        let workspace = tempfile::tempdir().unwrap();
        let agent_file = workspace.path().join("atlas.toml");
        std::fs::write(&agent_file, include_str!("../agents/atlas.toml")).unwrap();
        let mut saved = crate::settings::AtlasSettings::default();
        saved.deepseek.api_key = Some("test-placeholder".into());
        let settings = saved.resolve_with(|_| None).unwrap();
        let runtime =
            super::build_runtime_with_settings(workspace.path(), &agent_file, &settings).unwrap();
        for name in ["create_skill", "preview_skill", "install_skill"] {
            assert!(runtime
                .tool_definitions()
                .iter()
                .any(|tool| tool.name == name));
        }
    }
}
