use std::path::PathBuf;

use atlas_agentic::{
    conversations::ConversationId,
    skills::{self, SkillCatalog, SkillDocument, SkillInfo, SkillPreview, SkillScope, SkillSource},
};
use serde::Deserialize;
use tauri::State;

use crate::commands::{run_blocking, DesktopState};

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SkillWorkspaceContext {
    conversation_id: Option<ConversationId>,
    workspace_path: Option<PathBuf>,
}

#[tauri::command]
pub async fn list_skills(
    context: SkillWorkspaceContext,
    state: State<'_, DesktopState>,
) -> Result<SkillCatalog, String> {
    let state = state.inner().clone();
    run_blocking("desktop.list_skills", move || {
        state
            .skills_store(
                context.conversation_id.as_ref(),
                context.workspace_path.as_deref(),
            )?
            .list()
    })
    .await
}

#[tauri::command]
pub async fn read_skill(
    id: String,
    context: SkillWorkspaceContext,
    state: State<'_, DesktopState>,
) -> Result<SkillDocument, String> {
    let state = state.inner().clone();
    run_blocking("desktop.read_skill", move || {
        state
            .skills_store(
                context.conversation_id.as_ref(),
                context.workspace_path.as_deref(),
            )?
            .read(&id, None)
    })
    .await
}

#[tauri::command]
pub async fn preview_skill_import(url: String) -> Result<Vec<SkillPreview>, String> {
    run_blocking("desktop.preview_skill_import", move || {
        skills::github::preview(&url)
    })
    .await
}

#[tauri::command]
pub async fn install_skill(
    source: SkillSource,
    scope: SkillScope,
    replace: bool,
    context: SkillWorkspaceContext,
    state: State<'_, DesktopState>,
) -> Result<SkillInfo, String> {
    let state = state.inner().clone();
    run_blocking("desktop.install_skill", move || {
        let store = state.skills_store(
            context.conversation_id.as_ref(),
            context.workspace_path.as_deref(),
        )?;
        let package = skills::github::download(&source)?;
        store.install(package, scope, replace)
    })
    .await
}

#[tauri::command]
pub async fn set_skill_enabled(
    id: String,
    enabled: bool,
    context: SkillWorkspaceContext,
    state: State<'_, DesktopState>,
) -> Result<SkillInfo, String> {
    let state = state.inner().clone();
    run_blocking("desktop.set_skill_enabled", move || {
        state
            .skills_store(
                context.conversation_id.as_ref(),
                context.workspace_path.as_deref(),
            )?
            .set_enabled(&id, enabled)
    })
    .await
}
