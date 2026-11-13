//! Share immutable tool configuration; move the exact authorization snapshot to one worker.
use super::ExecutionReporter;
use crate::{
    permissions::ToolRisk,
    tools::{Tool, ToolDefinition, ToolResult},
};
use std::{any::Any, sync::Arc};
pub type Work = Box<dyn FnOnce(ExecutionReporter) -> Result<ToolResult, String> + Send>;
pub fn tool<T: Tool + Send + Sync + 'static>(tool: T) -> Box<dyn Tool> {
    Box::new(WorkerTool(Arc::new(tool)))
}
struct WorkerTool<T>(Arc<T>);
impl<T: Tool + Send + Sync + 'static> Tool for WorkerTool<T> {
    fn name(&self) -> &str {
        self.0.name()
    }
    fn has_execution_worker(&self) -> bool {
        true
    }
    fn definition(&self) -> ToolDefinition {
        self.0.definition()
    }
    fn reset_request_state(&self) {
        self.0.reset_request_state();
    }
    fn risk(&self, args: &serde_json::Value) -> Result<ToolRisk, String> {
        self.0.risk(args)
    }
    fn summarize(&self, args: &serde_json::Value) -> String {
        self.0.summarize(args)
    }
    fn authorize(
        &self,
        args: &serde_json::Value,
    ) -> Result<(ToolRisk, Option<Box<dyn Any + Send>>), String> {
        self.0.authorize(args)
    }
    fn execute(&self, args: &serde_json::Value) -> Result<ToolResult, String> {
        self.0.execute(args)
    }
    fn execute_authorized(
        &self,
        args: &serde_json::Value,
        context: Option<&dyn Any>,
    ) -> Result<ToolResult, String> {
        self.0.execute_authorized(args, context)
    }
    fn prepare_execution(
        &self,
        args: serde_json::Value,
        context: Option<Box<dyn Any + Send>>,
    ) -> Result<Work, String> {
        let tool = self.0.clone();
        Ok(Box::new(move |reporter| {
            let mut result =
                tool.execute_authorized(&args, context.as_deref().map(|c| c as &dyn Any))?;
            if tool.name() == "exec_command" {
                publish_process(&reporter, &result);
            }
            let mut stop_sent = false;
            loop {
                if reporter.cancelled() && !stop_sent {
                    stop_sent = tool.cancel_execution(&result)?;
                    if stop_sent {
                        reporter.ack_cancel();
                    }
                }
                let Some(next) = tool.poll_execution(&result, 100)? else {
                    break;
                };
                if tool.name() == "exec_command" {
                    publish_process(&reporter, &next);
                }
                result = next;
            }
            Ok(result)
        }))
    }
}
fn publish_process(reporter: &ExecutionReporter, result: &ToolResult) {
    let Ok(mut value) = serde_json::from_str::<serde_json::Value>(&result.message) else {
        return;
    };
    if let Some(text) = value.get("stdout").and_then(|v| v.as_str()) {
        reporter.append(text);
    }
    if let Some(text) = value
        .get("stderr")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
    {
        reporter.append("\n[stderr]\n");
        reporter.append(text);
    }
    if value.get("running").and_then(|v| v.as_bool()) != Some(true)
        && value.get("exit_code").is_some()
    {
        let status = if value.get("stopped") == Some(&serde_json::Value::Bool(true)) {
            "stopped"
        } else if value.get("timed_out") == Some(&serde_json::Value::Bool(true)) {
            "timedOut"
        } else if value.get("exit_code").and_then(|v| v.as_i64()) != Some(0)
            || value.get("error").is_some_and(|v| !v.is_null())
        {
            "failed"
        } else {
            "exited"
        };
        value["status"] = serde_json::json!(status);
    }
    if let Some(error) = value
        .get("error")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
    {
        reporter.append("\n[error]\n");
        reporter.append(error);
    }
    if let Some(obj) = value.as_object_mut() {
        obj.remove("stdout");
        obj.remove("stderr");
        obj.remove("error");
    }
    reporter.metadata(value);
}
