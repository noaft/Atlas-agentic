//! Child run operations derive ownership from the same native chat scopes as commands.
use crate::{
    commands::{run_blocking, DesktopState},
    process_commands::{resolve, ProcessContext},
};
use atlas_agentic::agent_runs::AgentRunView;
use tauri::{State, WebviewWindow};
#[tauri::command]
pub async fn list_agent_runs(
    client: String,
    context: ProcessContext,
    window: WebviewWindow,
    state: State<'_, DesktopState>,
) -> Result<Vec<AgentRunView>, String> {
    let state = state.inner().clone();
    let label = window.label().to_string();
    run_blocking("desktop.agent_runs", move || {
        let scope = resolve(&state, &label, &client, &context)?;
        state.processes.agents.list(scope.id())
    })
    .await
}
#[tauri::command]
pub async fn cancel_agent_run(
    client: String,
    context: ProcessContext,
    run_id: String,
    window: WebviewWindow,
    state: State<'_, DesktopState>,
) -> Result<AgentRunView, String> {
    let state = state.inner().clone();
    let label = window.label().to_string();
    run_blocking("desktop.cancel_agent", move || {
        let scope = resolve(&state, &label, &client, &context)?;
        state.processes.agents.cancel(scope.id(), &run_id)
    })
    .await
}
#[tauri::command]
pub async fn resolve_agent_run_approval(
    client: String,
    context: ProcessContext,
    run_id: String,
    approval_id: String,
    allow: bool,
    window: WebviewWindow,
    state: State<'_, DesktopState>,
) -> Result<(), String> {
    let state = state.inner().clone();
    let label = window.label().to_string();
    run_blocking("desktop.agent_approval", move || {
        let scope = resolve(&state, &label, &client, &context)?;
        state
            .processes
            .agents
            .resolve_approval(scope.id(), &run_id, &approval_id, allow)
    })
    .await
}
