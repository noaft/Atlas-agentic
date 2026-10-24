use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PermissionMode {
    #[default]
    Ask,
    Workspace,
    FullAccess,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolRisk {
    ReadOnly,
    ModifyWorkspace,
    ModifyOutsideWorkspace,
    Destructive,
    Launch,
    Command,
    ReadOutsideWorkspace,
    ComputerObserve,
    ComputerControl,
    ExternalTool,
}

impl ToolRisk {
    pub fn reason(self) -> &'static str {
        match self {
            Self::ReadOnly => "reads information without changing the system",
            Self::ModifyWorkspace => "changes files inside the workspace",
            Self::ModifyOutsideWorkspace => "changes files outside the selected workspace",
            Self::Destructive => "deletes or overwrites data",
            Self::Launch => "opens a file or application on the computer",
            Self::Command => "executes a shell command with the Atlas process permissions",
            Self::ReadOutsideWorkspace => {
                "reads files outside the selected workspace and may send sensitive content to the configured model provider"
            }
            Self::ComputerObserve => {
                "reads potentially sensitive desktop information that may be sent to the configured model provider"
            }
            Self::ComputerControl => "sends mouse or keyboard input to the active application",
            Self::ExternalTool => "calls an external MCP tool that may launch a process, access sensitive data, or change external state",
        }
    }
}

pub fn requires_confirmation(mode: PermissionMode, risk: ToolRisk) -> bool {
    match mode {
        PermissionMode::FullAccess => false,
        PermissionMode::Ask => risk != ToolRisk::ReadOnly,
        PermissionMode::Workspace => matches!(
            risk,
            ToolRisk::ModifyOutsideWorkspace
                | ToolRisk::Destructive
                | ToolRisk::Launch
                | ToolRisk::Command
                | ToolRisk::ReadOutsideWorkspace
                | ToolRisk::ComputerObserve
                | ToolRisk::ComputerControl
                | ToolRisk::ExternalTool
        ),
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AuthorizationRequest {
    pub id: String,
    pub tool: String,
    pub arguments: String,
    pub reason: String,
}

impl AuthorizationRequest {
    pub fn new(tool: &str, arguments: String, risk: ToolRisk) -> Self {
        Self {
            id: Uuid::new_v4().to_string(),
            tool: tool.to_string(),
            arguments,
            reason: risk.reason().to_string(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthorizationDecision {
    Allow,
    Deny,
}

pub trait ToolAuthorizer {
    fn authorize(&self, request: &AuthorizationRequest) -> Result<AuthorizationDecision, String>;
}

pub struct AllowAllAuthorizer;

impl ToolAuthorizer for AllowAllAuthorizer {
    fn authorize(&self, _request: &AuthorizationRequest) -> Result<AuthorizationDecision, String> {
        Ok(AuthorizationDecision::Allow)
    }
}

pub struct MissingAuthorizer;

impl ToolAuthorizer for MissingAuthorizer {
    fn authorize(&self, request: &AuthorizationRequest) -> Result<AuthorizationDecision, String> {
        Err(format!(
            "tool {} requires an explicit authorizer",
            request.tool
        ))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum RuntimeStatus {
    Thinking,
    PreparingAnswer,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "event", content = "data", rename_all = "camelCase")]
pub enum ToolEvent {
    /// Transient, cumulative display text. None discards the preview.
    ProgressPreview {
        message: Option<String>,
    },
    Progress {
        message: String,
    },
    ResponseReset,
    Compaction {
        progress: crate::conversations::CompactionProgress,
    },
    ContextStatus {
        status: crate::runtime::context::ContextStatus,
    },
    Usage {
        usage: Option<crate::models::ModelUsage>,
    },
    Status {
        status: RuntimeStatus,
    },
    Requested {
        sequence: u32,
        tool: String,
        arguments: String,
    },
    ApprovalRequested {
        sequence: u32,
        request: AuthorizationRequest,
    },
    Denied {
        sequence: u32,
        tool: String,
        summary: String,
    },
    Started {
        sequence: u32,
        tool: String,
    },
    Finished {
        sequence: u32,
        tool: String,
        success: bool,
        summary: String,
        details: Option<String>,
    },
}
