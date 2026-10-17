//! Provider-independent built-in tasks shared by CLI and desktop.
use std::{collections::HashSet, sync::atomic::AtomicBool};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::conversations::{
    CompactionProgress, CompactionResult, ConversationId, ConversationService,
};

pub const COMPACT_TASK_ID: &str = "conversation.compact";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskDescriptor {
    pub id: String,
    pub command: String,
    pub title: String,
    pub description: String,
    pub argument_schema: Value,
    pub requires_conversation: bool,
    pub requires_model: bool,
    pub cancellable: bool,
}

#[derive(Debug, Clone, Copy)]
pub enum TaskHandler {
    Compact,
}

#[derive(Debug, Clone)]
pub struct TaskDefinition {
    pub descriptor: TaskDescriptor,
    pub handler: TaskHandler,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TaskRequest {
    pub task_id: String,
    pub arguments: Value,
    pub conversation_id: Option<ConversationId>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskResult {
    pub task_id: String,
    pub outcome: TaskOutcome,
}

#[derive(Debug, Serialize)]
#[serde(tag = "kind", content = "result", rename_all = "camelCase")]
pub enum TaskOutcome {
    Compaction(CompactionResult),
}

pub struct TaskRegistry {
    definitions: Vec<TaskDefinition>,
}

impl TaskRegistry {
    pub fn new(definitions: Vec<TaskDefinition>) -> Result<Self, String> {
        let mut ids = HashSet::new();
        let mut commands = HashSet::new();
        for definition in &definitions {
            let descriptor = &definition.descriptor;
            if !ids.insert(descriptor.id.clone())
                || !commands.insert(descriptor.command.to_ascii_lowercase())
            {
                return Err("Duplicate task ID or command".into());
            }
        }
        Ok(Self { definitions })
    }

    pub fn builtins() -> Self {
        Self::new(vec![TaskDefinition {
            descriptor: TaskDescriptor {
                id: COMPACT_TASK_ID.into(),
                command: "compact".into(),
                title: "Compact conversation".into(),
                description: "Summarize older context while keeping the latest messages.".into(),
                argument_schema: json!({"type": "object", "properties": {}, "additionalProperties": false}),
                requires_conversation: true,
                requires_model: true,
                cancellable: true,
            },
            handler: TaskHandler::Compact,
        }])
        .expect("built-in task definitions must be unique")
    }

    pub fn descriptors(&self) -> Vec<TaskDescriptor> {
        self.definitions
            .iter()
            .map(|d| d.descriptor.clone())
            .collect()
    }

    pub fn validate(&self, request: &TaskRequest) -> Result<TaskHandler, String> {
        let definition = self
            .definitions
            .iter()
            .find(|d| d.descriptor.id == request.task_id)
            .ok_or_else(|| format!("Unknown task: {}", request.task_id))?;
        match definition.handler {
            TaskHandler::Compact => {
                if !request
                    .arguments
                    .as_object()
                    .is_some_and(|args| args.is_empty())
                {
                    return Err("Invalid task arguments: expected an empty object".into());
                }
            }
        }
        if definition.descriptor.requires_conversation && request.conversation_id.is_none() {
            return Err("Start a saved conversation first".into());
        }
        Ok(definition.handler)
    }

    pub fn run(
        &self,
        request: TaskRequest,
        service: &ConversationService<'_>,
        cancelled: Option<&AtomicBool>,
        on_progress: &mut dyn FnMut(CompactionProgress),
    ) -> Result<TaskResult, String> {
        let outcome = match self.validate(&request)? {
            TaskHandler::Compact => TaskOutcome::Compaction(
                service.compact_cancellable(
                    request
                        .conversation_id
                        .as_ref()
                        .ok_or("Start a saved conversation first")?,
                    on_progress,
                    cancelled,
                )?,
            ),
        };
        Ok(TaskResult {
            task_id: request.task_id,
            outcome,
        })
    }
}
