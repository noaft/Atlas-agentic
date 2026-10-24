//! The frontend receives only connection status, never credentials or OAuth URLs.
use atlas_agentic::mcp::oauth::{self, ConnectionStatus};
use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex, OnceLock,
    },
};

static PENDING_CONNECTIONS: OnceLock<Mutex<HashMap<String, Arc<AtomicBool>>>> = OnceLock::new();

fn pending_connections() -> &'static Mutex<HashMap<String, Arc<AtomicBool>>> {
    PENDING_CONNECTIONS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn validate_server_name(server: &str) -> Result<(), String> {
    if server.is_empty()
        || server.len() > 64
        || !server
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.'))
    {
        return Err(
            "MCP server name must contain 1–64 ASCII letters, digits, _, -, or .".to_string(),
        );
    }
    Ok(())
}

struct PendingConnection {
    server: String,
    canceled: Arc<AtomicBool>,
}

impl PendingConnection {
    fn reserve(server: &str) -> Result<Self, String> {
        validate_server_name(server)?;
        let mut pending = pending_connections()
            .lock()
            .map_err(|_| "MCP pending connection registry is unavailable".to_string())?;
        if pending.contains_key(server) {
            return Err("An MCP connection is already pending for this server".to_string());
        }
        let canceled = Arc::new(AtomicBool::new(false));
        pending.insert(server.to_string(), Arc::clone(&canceled));
        Ok(Self {
            server: server.to_string(),
            canceled,
        })
    }
}

impl Drop for PendingConnection {
    fn drop(&mut self) {
        let mut pending = pending_connections()
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if pending
            .get(&self.server)
            .is_some_and(|flag| Arc::ptr_eq(flag, &self.canceled))
        {
            pending.remove(&self.server);
        }
    }
}

fn cancel_pending(server: &str) -> Result<(), String> {
    validate_server_name(server)?;
    let pending = pending_connections()
        .lock()
        .map_err(|_| "MCP pending connection registry is unavailable".to_string())?;
    if let Some(canceled) = pending.get(server) {
        canceled.store(true, Ordering::SeqCst);
    }
    Ok(())
}

#[tauri::command]
pub async fn list_mcp_connections() -> Result<Vec<ConnectionStatus>, String> {
    tauri::async_runtime::spawn_blocking(oauth::list_connections)
        .await
        .map_err(|_| "MCP connection status worker failed".to_string())?
}

#[tauri::command]
pub async fn connect_mcp_server(server: String) -> Result<(), String> {
    let pending = PendingConnection::reserve(&server)?;
    tauri::async_runtime::spawn_blocking(move || {
        let result = oauth::connect_server_with_cancel(&server, Arc::clone(&pending.canceled));
        drop(pending);
        result
    })
    .await
    .map_err(|_| "MCP connection worker failed".to_string())?
}

#[tauri::command]
pub async fn disconnect_mcp_server(server: String) -> Result<(), String> {
    cancel_pending(&server)?;
    tauri::async_runtime::spawn_blocking(move || oauth::disconnect_server(&server))
        .await
        .map_err(|_| "MCP disconnection worker failed".to_string())?
}

#[tauri::command]
pub async fn cancel_mcp_connection(server: String) -> Result<(), String> {
    cancel_pending(&server)?;
    tauri::async_runtime::spawn_blocking(move || oauth::cancel_connection(&server))
        .await
        .map_err(|_| "MCP cancellation worker failed".to_string())?
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cancellation_before_worker_start_sets_shared_flag_and_reservation_releases() {
        let name = "desktop_cancel_before_worker";
        let pending = PendingConnection::reserve(name).unwrap();
        let worker_flag = Arc::clone(&pending.canceled);
        assert!(!worker_flag.load(Ordering::SeqCst));
        assert!(PendingConnection::reserve(name).is_err());
        cancel_pending(name).unwrap();
        assert!(worker_flag.load(Ordering::SeqCst));
        drop(pending);
        let replacement = PendingConnection::reserve(name).unwrap();
        assert!(!replacement.canceled.load(Ordering::SeqCst));
    }

    #[test]
    fn pending_commands_validate_names_without_loading_configuration() {
        for invalid in ["", "../server", "server with spaces", "é"] {
            assert!(PendingConnection::reserve(invalid).is_err());
            assert!(cancel_pending(invalid).is_err());
        }
        assert!(PendingConnection::reserve(&"x".repeat(65)).is_err());
        assert!(PendingConnection::reserve("valid-server_1.test").is_ok());
    }
}
