use crate::{commands::DesktopState, process_commands::ProcessContext};
use atlas_agentic::turns::InputReceipt;

pub(crate) fn submit_impl(
    state: &DesktopState,
    window: &str,
    client: &str,
    context: &ProcessContext,
    request_id: &str,
    input_id: &str,
    content: &str,
) -> Result<InputReceipt, String> {
    let scope = crate::process_commands::resolve(state, window, client, context)?;
    state
        .processes
        .turns
        .submit(scope.id(), request_id, input_id, content)
}

#[cfg(test)]
pub(crate) fn stop_impl(
    state: &DesktopState,
    window: &str,
    client: &str,
    context: &ProcessContext,
    request_id: &str,
) -> Result<(), String> {
    stop_with_cursor_impl(state, window, client, context, request_id, None)
}

fn stop_with_cursor_impl(
    state: &DesktopState,
    window: &str,
    client: &str,
    context: &ProcessContext,
    request_id: &str,
    cursor: Option<atlas_agentic::turns::DisplayCursor>,
) -> Result<(), String> {
    let scope = crate::process_commands::resolve(state, window, client, context)?;
    state
        .processes
        .turns
        .cancel_with_cursor(scope.id(), request_id, cursor)
}

#[tauri::command]
pub async fn submit_chat_input(
    client: String,
    context: ProcessContext,
    request_id: String,
    input_id: String,
    content: String,
    window: tauri::WebviewWindow,
    state: tauri::State<'_, DesktopState>,
) -> Result<InputReceipt, String> {
    let state = state.inner().clone();
    let label = window.label().to_owned();
    crate::commands::run_blocking("desktop.submit_input", move || {
        submit_impl(
            &state,
            &label,
            &client,
            &context,
            &request_id,
            &input_id,
            &content,
        )
    })
    .await
}

#[tauri::command]
pub async fn stop_chat_request(
    client: String,
    context: ProcessContext,
    request_id: String,
    response_epoch: Option<u64>,
    displayed_bytes: Option<usize>,
    window: tauri::WebviewWindow,
    state: tauri::State<'_, DesktopState>,
) -> Result<(), String> {
    let cursor = match (response_epoch, displayed_bytes) {
        (Some(response_epoch), Some(displayed_bytes)) => {
            Some(atlas_agentic::turns::DisplayCursor {
                response_epoch,
                displayed_bytes,
            })
        }
        (None, None) => None,
        _ => {
            return Err("Provide both responseEpoch and displayedBytes for a Stop snapshot".into())
        }
    };
    let state = state.inner().clone();
    let label = window.label().to_owned();
    crate::commands::run_blocking("desktop.stop_chat", move || {
        stop_with_cursor_impl(&state, &label, &client, &context, &request_id, cursor)
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use atlas_agentic::{conversations::ConversationStore, tools::ProcessScope};
    fn fixture() -> (tempfile::TempDir, DesktopState, String, ProcessScope) {
        let dir = tempfile::tempdir().unwrap();
        let state = DesktopState::new(ConversationStore::at(dir.path().into()));
        let client = state.processes.initialize("main").unwrap();
        let scope = state
            .processes
            .resolve(
                "main",
                &client,
                &ProcessContext {
                    conversation_id: None,
                    scope_id: None,
                },
            )
            .unwrap();
        (dir, state, client, scope)
    }
    #[test]
    fn live_input_and_stop_require_the_native_owner_and_exact_request() {
        let (_dir, state, client, scope) = fixture();
        let other = state
            .processes
            .resolve(
                "main",
                &client,
                &ProcessContext {
                    conversation_id: None,
                    scope_id: None,
                },
            )
            .unwrap();
        let a = state.processes.turns.begin(scope.id()).unwrap();
        let b = state.processes.turns.begin(other.id()).unwrap();
        let context = ProcessContext {
            conversation_id: None,
            scope_id: Some(scope.id().into()),
        };
        assert!(submit_impl(
            &state,
            "main",
            &client,
            &context,
            b.control().request_id(),
            "input",
            "wrong target"
        )
        .is_err());
        let receipt = submit_impl(
            &state,
            "main",
            &client,
            &context,
            a.control().request_id(),
            "input",
            "new direction",
        )
        .unwrap();
        assert!(receipt.accepted);
        assert_eq!(a.control().drain().len(), 1);
        assert!(b.control().drain().is_empty());
        assert!(stop_impl(&state, "main", &client, &context, b.control().request_id()).is_err());
        assert!(!b
            .control()
            .cancelled()
            .load(std::sync::atomic::Ordering::SeqCst));
        stop_impl(&state, "main", &client, &context, a.control().request_id()).unwrap();
        assert!(a
            .control()
            .cancelled()
            .load(std::sync::atomic::Ordering::SeqCst));
    }
    #[test]
    fn expired_and_foreign_client_requests_cannot_reach_the_successor() {
        let (_dir, state, client, scope) = fixture();
        let context = ProcessContext {
            conversation_id: None,
            scope_id: Some(scope.id().into()),
        };
        let first = state.processes.turns.begin(scope.id()).unwrap();
        let stale = first.control().request_id().to_owned();
        drop(first);
        let next = state.processes.turns.begin(scope.id()).unwrap();
        assert!(submit_impl(&state, "main", &client, &context, &stale, "old", "late").is_err());
        assert!(submit_impl(
            &state,
            "other-window",
            &client,
            &context,
            next.control().request_id(),
            "foreign",
            "wrong window"
        )
        .is_err());
        assert!(next.control().drain().is_empty());
    }

    #[test]
    fn native_stop_forwards_the_visible_utf8_prefix_to_the_owned_request() {
        let (_dir, state, client, scope) = fixture();
        let lease = state.processes.turns.begin(scope.id()).unwrap();
        let control = lease.control();
        control.record_displayed_delta("visible é");
        let epoch = control.response_epoch();
        control.record_displayed_delta(" unseen late chunk");
        let context = ProcessContext {
            conversation_id: None,
            scope_id: Some(scope.id().into()),
        };
        stop_with_cursor_impl(
            &state,
            "main",
            &client,
            &context,
            control.request_id(),
            Some(atlas_agentic::turns::DisplayCursor {
                response_epoch: epoch,
                displayed_bytes: "visible é".len(),
            }),
        )
        .unwrap();
        assert_eq!(
            control.stopped_prefix().unwrap().as_deref(),
            Some("visible é")
        );
    }
}
