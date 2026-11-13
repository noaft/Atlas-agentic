//! Deterministic Wakeup walkthrough, without a model service or API key.
use atlas_agentic::{
    agents::{AgentRegistry, LocalAgent, LocalAgentDefinition},
    executions::worker::Work,
    models::{ModelProvider, ModelRequest},
    permissions::{AllowAllAuthorizer, PermissionMode},
    tools::{Tool, ToolDefinition, ToolRegistry, ToolResult},
    Runtime,
};
use std::{cell::Cell, thread, time::Duration};
struct Ticker;
impl Tool for Ticker {
    fn name(&self) -> &str {
        "demo_ticker"
    }
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: self.name().into(),
            description: "In-memory demo ticker".into(),
            input_schema: serde_json::json!({"type":"object"}),
        }
    }
    fn execute(&self, _: &serde_json::Value) -> Result<ToolResult, String> {
        Ok(ToolResult::text("Use the Wakeup policy for this demo"))
    }
    fn prepare_execution(
        &self,
        _: serde_json::Value,
        _: Option<Box<dyn std::any::Any + Send>>,
    ) -> Result<Work, String> {
        Ok(Box::new(|reporter| {
            for n in 1..=10 {
                if reporter.cancelled() {
                    reporter.ack_cancel();
                    return Ok(ToolResult::text("cancelled"));
                }
                reporter.append(&format!("tick {n}\n"));
                thread::sleep(Duration::from_millis(120));
            }
            Ok(ToolResult::text("done"))
        }))
    }
}
struct DemonstrationAgent {
    started: Cell<bool>,
    waits: Cell<u64>,
}
impl ModelProvider for DemonstrationAgent {
    fn complete(&self, request: &ModelRequest) -> Result<String, String> {
        if !self.started.replace(true) {
            return Ok(r#"{"type":"tool_call","tool":"demo_ticker","wake_up":{"mode":"hard","interval_seconds":0.2},"args":{}}"#.into());
        }
        let input: serde_json::Value =
            serde_json::from_str(request.user_prompt).map_err(|_| "Invalid continuation")?;
        let info = &input["executions"][0];
        let id = info["execution_id"].as_str().ok_or("Missing ID")?;
        let records = input["tool_results"].as_array().ok_or("Missing records")?;
        if input["execution_wakeup"].is_object() {
            println!(
                "Wakeup: status={}, unread lines={} (no log text in notification)",
                info["status"], info["unread_lines"]
            );
            return Ok(format!(
                r#"{{"type":"read_execution","execution_id":"{id}","tail_lines":3}}"#
            ));
        }
        if let Some(read) = records.last().filter(|v| v["tool"] == "read_execution") {
            let selected: serde_json::Value =
                serde_json::from_str(read["output"].as_str().ok_or("Missing read")?)
                    .map_err(|_| "Invalid read")?;
            println!(
                "Selected last 3 lines:\n{}",
                selected["result"]["text"].as_str().unwrap_or("")
            );
            if info["status"] == "succeeded" {
                return Ok(r#"{"type":"final","content":"PASS: bounded reads, adjustable wakeup, completion wakeup, one execution."}"#.into());
            }
        }
        let phase = self.waits.get();
        self.waits.set(phase + 1);
        if phase == 0 || phase == 2 {
            let seconds = if phase == 0 { 0.5 } else { 1.0 };
            println!("Agent changes the wakeup interval to {seconds}s without repeating the tool.");
            return Ok(format!(
                r#"{{"type":"set_execution_wakeup","execution_id":"{id}","wake_up":{{"mode":"hard","interval_seconds":{seconds}}}}}"#
            ));
        }
        println!("Agent awaits the scheduled event; runtime waits without model calls.");
        Ok(format!(
            r#"{{"type":"await_execution","execution_id":"{id}"}}"#
        ))
    }
}
fn main() -> Result<(), String> {
    let definition=LocalAgentDefinition::from_toml("name='demo'\ndisplay_name='Demo'\nmodel_provider='fake'\nmodel='fake'\nsystem_prompt='Deterministic demonstration'\nconversation_title_prompt='Demo'")?;
    let runtime = Runtime::new(AgentRegistry::new(vec![Box::new(LocalAgent::new(
        definition,
        Box::new(DemonstrationAgent {
            started: Cell::new(false),
            waits: Cell::new(0),
        }),
    ))]))
    .with_tools(ToolRegistry::new(vec![Box::new(Ticker)]));
    let result = runtime.ask_stream_with_history_and_authorization(
        "Demonstrate Wakeup",
        &[],
        PermissionMode::Ask,
        &AllowAllAuthorizer,
        &mut |_| {},
        &mut |_| {},
    )?;
    println!("{result}");
    Ok(())
}
