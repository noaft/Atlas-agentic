//! Run with `cargo run --example process_sessions`; no provider or credentials.
use atlas_agentic::{
    agents::{AgentRegistry, LocalAgent, LocalAgentDefinition},
    models::{ModelProvider, ModelRequest},
    permissions::{AllowAllAuthorizer, PermissionMode},
    tools::{ProcessCursor, ProcessManager},
    Runtime,
};
use std::{cell::Cell, time::Duration};
struct DemoModel {
    response: String,
    phase: Cell<bool>,
}
impl ModelProvider for DemoModel {
    fn complete(&self, request: &ModelRequest) -> Result<String, String> {
        if !self.phase.replace(true) {
            Ok(self.response.clone())
        } else {
            let pending: serde_json::Value = serde_json::from_str(request.user_prompt)
                .map_err(|_| "Invalid demo continuation")?;
            let record = pending["tool_results"]
                .as_array()
                .and_then(|records| records.last())
                .ok_or("Missing demo tool result")?;
            if record["success"] != true {
                return Err("Demo tool failed".into());
            }
            let output: serde_json::Value =
                serde_json::from_str(record["output"].as_str().ok_or("Missing demo output")?)
                    .map_err(|_| "Invalid demo output")?;
            if output["running"] != true {
                return Err("Demo worker did not stay running".into());
            }
            Ok(r#"{"type":"final","content":"Demo step complete."}"#.into())
        }
    }
}
fn runtime(command: String) -> Runtime {
    let definition=LocalAgentDefinition::from_toml("name='demo'\ndisplay_name='Demo'\nmodel_provider='fake'\nmodel='fake'\nsystem_prompt='Deterministic local process demo.'\nconversation_title_prompt='Title.'").unwrap();
    Runtime::new(AgentRegistry::new(vec![Box::new(LocalAgent::new(
        definition,
        Box::new(DemoModel {
            response: command,
            phase: Cell::new(false),
        }),
    ))]))
}
fn ask(runtime: &Runtime) -> Result<(), String> {
    runtime
        .ask_stream_with_history_and_authorization(
            "controlled demo",
            &[],
            PermissionMode::FullAccess,
            &AllowAllAuthorizer,
            &mut |_| {},
            &mut |_| {},
        )
        .map(|_| ())
}
fn main() -> Result<(), String> {
    if std::env::args().nth(1).as_deref() == Some("--worker") {
        for index in 0..600 {
            println!("tick {index}: background is alive");
            std::thread::sleep(Duration::from_millis(100));
        }
        return Ok(());
    }
    let manager = ProcessManager::new();
    let scope = manager.create_scope()?;
    let other = manager.create_scope()?;
    let workspace = std::env::current_dir().map_err(|e| e.to_string())?;
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    #[cfg(unix)]
    let command = format!(
        "'{}' --worker",
        exe.to_string_lossy().replace('\'', "'\\''")
    );
    #[cfg(windows)]
    let command = format!("\"{}\" --worker", exe.display());
    let args = serde_json::json!({"command":command,"lifetime":"background","yield_time_ms":100});
    let first = runtime(format!(
        r#"{{"type":"tool_call","tool":"exec_command","args":{args}}}"#
    ))
    .with_process_scope(&workspace, scope.clone());
    ask(&first)?;
    drop(first);
    let info = scope
        .list()?
        .into_iter()
        .next()
        .ok_or("Demo process missing")?;
    let id = info.session_id;
    println!(
        "1. First runtime ended; background {id} is still running: {}",
        scope.list()?[0].running
    );
    std::thread::sleep(Duration::from_millis(200));
    let viewer = scope.read(&id, Some(ProcessCursor::default()), 0)?;
    println!("2. Independent UI reader sees: {}", viewer.stdout.trim());
    let args = serde_json::json!({"session_id":id,"wait_ms":0});
    let second = runtime(format!(
        r#"{{"type":"tool_call","tool":"read_process","args":{args}}}"#
    ))
    .with_process_scope(&workspace, scope.clone());
    ask(&second)?;
    drop(second);
    println!(
        "3. Second runtime inspected same session; resources: {}",
        scope.list()?.len()
    );
    if other.read(&id, Some(ProcessCursor::default()), 0).is_ok() {
        return Err("Ownership check failed".into());
    }
    println!("4. Another chat cannot read this session: PASS");
    let stopped = scope.stop(&id)?;
    if stopped.running {
        return Err("Stop still pending".into());
    }
    println!(
        "5. Explicit stop: stopped={}, running={}",
        stopped.stopped, stopped.running
    );
    manager.shutdown(Duration::from_secs(3))?;
    println!("PASS: process reused across runtimes, isolated by chat, and cleaned up.");
    Ok(())
}
