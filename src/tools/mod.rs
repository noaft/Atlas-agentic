mod agent_catalog;
pub use agent_catalog::register_agent_catalog_tools;
mod apply_patch;
mod computer;
mod copy_file;
mod create_directory;
mod create_file;
mod create_skill;
mod current_time;
mod delete_file;
#[cfg(unix)]
mod exec_command;
#[cfg(windows)]
mod exec_command_windows;
mod import_skill;
mod inspection;
mod list_dir;
mod load_skill;
mod move_file;
mod open_application;
mod open_file;
#[cfg(any(windows, test))]
mod output_capture;
mod platform;
#[cfg(any(unix, windows))]
mod process_sessions;
mod pwd;
mod read_file;
mod ripgrep;
mod search_files;
mod workspace_path;

pub use apply_patch::ApplyPatchTool;
pub use computer::{
    system_computer_controller, ActiveWindowState, CaptureScreenTool, CapturedImage,
    ComputerController, ComputerKey, ComputerUiState, CoordinateMode, GetUiStateTool, KeyDirection,
    KeyPressTool, MouseButton, MouseClickTool, MouseMoveTool, MouseScrollTool, Observation, Point,
    ScrollAxis, SharedComputerController, Size, TypeTextTool, UnavailableState, WindowBounds,
};
pub use copy_file::CopyFileTool;
pub use create_directory::CreateDirectoryTool;
pub use create_file::CreateFileTool;
pub use create_skill::CreateSkillTool;
pub use current_time::CurrentTimeTool;
pub use delete_file::DeleteFileTool;
#[cfg(unix)]
pub use exec_command::ExecCommandTool;
#[cfg(windows)]
pub use exec_command_windows::ExecCommandTool;
pub use import_skill::register_skill_import_tools;
pub use list_dir::ListDirTool;
pub use load_skill::LoadSkillTool;
pub use move_file::MoveFileTool;
pub use open_application::OpenApplicationTool;
pub use open_file::OpenFileTool;
#[cfg(windows)]
pub(crate) use platform::system_launcher;
#[doc(hidden)]
pub use platform::{application_command, file_command, CommandSpec, Platform, ProcessLauncher};
#[cfg(any(unix, windows))]
pub(crate) use process_sessions::register_request_process_tools;
#[cfg(any(unix, windows))]
pub use process_sessions::{
    register_process_tools, register_process_tools_in_scope, ProcessCursor, ProcessInfo,
    ProcessLifetime, ProcessManager, ProcessOutputView, ProcessRead, ProcessScope,
};
pub use pwd::PwdTool;
pub use read_file::ReadFileTool;
pub use ripgrep::RipgrepTool;
pub use search_files::SearchFilesTool;

use std::{any::Any, collections::HashMap};

use crate::agent_output::AgentOutput;
use crate::media::ImageData;
use crate::permissions::ToolRisk;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolDefinition {
    pub name: String,
    pub description: String,
    pub input_schema: serde_json::Value,
}

pub struct ToolResult {
    pub message: String,
    pub attachments: Vec<ToolAttachment>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToolAttachment {
    Image(ImageData),
}

impl ToolResult {
    pub fn text(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            attachments: Vec::new(),
        }
    }

    pub fn with_image(message: impl Into<String>, image: ImageData) -> Self {
        Self {
            message: message.into(),
            attachments: vec![ToolAttachment::Image(image)],
        }
    }
}

pub trait Tool {
    fn name(&self) -> &str;
    fn definition(&self) -> ToolDefinition;
    /// Clear transient request state at runtime entry and on every exit.
    /// Persistent configuration and frozen skill catalogs are unaffected.
    fn reset_request_state(&self) {}
    fn risk(&self, _args: &serde_json::Value) -> Result<ToolRisk, String> {
        Ok(ToolRisk::ReadOnly)
    }
    fn authorize(
        &self,
        args: &serde_json::Value,
    ) -> Result<(ToolRisk, Option<Box<dyn Any>>), String> {
        Ok((self.risk(args)?, None))
    }
    fn summarize(&self, args: &serde_json::Value) -> String {
        summarize_arguments(args)
    }
    fn execute(&self, args: &serde_json::Value) -> Result<ToolResult, String>;
    fn execute_authorized(
        &self,
        args: &serde_json::Value,
        _context: Option<&dyn Any>,
    ) -> Result<ToolResult, String> {
        self.execute(args)
    }
}

pub fn summarize_arguments(args: &serde_json::Value) -> String {
    const LIMIT: usize = 512;
    let sanitized = redact_value(args, None);
    let rendered = serde_json::to_string(&sanitized).unwrap_or_else(|_| "{}".to_string());
    if rendered.chars().count() <= LIMIT {
        return rendered;
    }
    let mut bounded = rendered.chars().take(LIMIT - 1).collect::<String>();
    bounded.push('…');
    bounded
}

fn redact_value(value: &serde_json::Value, key: Option<&str>) -> serde_json::Value {
    if key.is_some_and(secret_key) {
        return serde_json::Value::String("[redacted]".to_string());
    }
    match value {
        serde_json::Value::Object(values) => serde_json::Value::Object(
            values
                .iter()
                .map(|(key, value)| (key.clone(), redact_value(value, Some(key))))
                .collect(),
        ),
        serde_json::Value::Array(values) => serde_json::Value::Array(
            values
                .iter()
                .map(|value| redact_value(value, None))
                .collect(),
        ),
        serde_json::Value::String(value) if value.chars().count() > 160 => {
            let mut bounded = value.chars().take(159).collect::<String>();
            bounded.push('…');
            serde_json::Value::String(bounded)
        }
        _ => value.clone(),
    }
}

fn secret_key(key: &str) -> bool {
    let key = key.to_ascii_lowercase();
    ["key", "token", "password", "secret", "authorization"]
        .iter()
        .any(|part| key.contains(part))
}

pub struct ToolRegistry {
    tools: HashMap<String, Box<dyn Tool>>,
}

pub(crate) struct ToolRequestScope<'a>(&'a ToolRegistry);

impl Drop for ToolRequestScope<'_> {
    fn drop(&mut self) {
        self.0.reset_request_state();
    }
}

pub struct ToolAuthorization {
    tool: String,
    args: serde_json::Value,
    risk: ToolRisk,
    arguments: String,
    context: Option<Box<dyn Any>>,
}

impl ToolAuthorization {
    pub fn risk(&self) -> ToolRisk {
        self.risk
    }

    pub fn arguments(&self) -> &str {
        &self.arguments
    }
}

impl ToolRegistry {
    pub fn empty() -> Self {
        Self {
            tools: HashMap::new(),
        }
    }

    pub fn new(tools: Vec<Box<dyn Tool>>) -> Self {
        let mut registry = Self::empty();
        for tool in tools {
            registry.register(tool);
        }
        registry
    }

    pub fn register(&mut self, tool: Box<dyn Tool>) {
        self.tools.insert(tool.name().to_string(), tool);
    }

    pub fn execute(&self, output: &AgentOutput) -> Result<ToolResult, String> {
        match output {
            AgentOutput::Final { content } => Ok(ToolResult::text(content.clone())),
            AgentOutput::ToolCall { tool, args, .. } => self
                .tools
                .get(tool)
                .ok_or_else(|| format!("unknown tool: {tool}"))?
                .execute(args),
        }
    }

    pub fn execute_authorized(
        &self,
        output: &AgentOutput,
        authorization: &ToolAuthorization,
    ) -> Result<ToolResult, String> {
        match output {
            AgentOutput::Final { content } => Ok(ToolResult::text(content.clone())),
            AgentOutput::ToolCall { tool, args, .. } => {
                if tool != &authorization.tool || args != &authorization.args {
                    return Err("tool call changed after authorization".to_string());
                }
                self.tools
                    .get(tool)
                    .ok_or_else(|| format!("unknown tool: {tool}"))?
                    .execute_authorized(args, authorization.context.as_deref())
            }
        }
    }

    pub fn authorization(
        &self,
        tool_name: &str,
        args: &serde_json::Value,
    ) -> Result<ToolAuthorization, String> {
        let tool = self
            .tools
            .get(tool_name)
            .ok_or_else(|| format!("unknown tool: {tool_name}"))?;
        let (risk, context) = tool.authorize(args)?;
        Ok(ToolAuthorization {
            tool: tool_name.to_string(),
            args: args.clone(),
            risk,
            arguments: tool.summarize(args),
            context,
        })
    }

    fn reset_request_state(&self) {
        for tool in self.tools.values() {
            tool.reset_request_state();
        }
    }

    pub(crate) fn request_scope(&self) -> ToolRequestScope<'_> {
        self.reset_request_state();
        ToolRequestScope(self)
    }

    pub(crate) fn retain_names(&mut self, names: &[String]) {
        self.tools.retain(|name, _| names.contains(name));
    }

    pub(crate) fn remove(&mut self, name: &str) {
        self.tools.remove(name);
    }

    pub fn definitions(&self) -> Vec<ToolDefinition> {
        self.tools.values().map(|tool| tool.definition()).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::ToolResult;

    #[test]
    fn tool_result_text_has_no_attachments() {
        let result = ToolResult::text("done");

        assert_eq!(result.message, "done");
        assert!(result.attachments.is_empty());
    }
}
