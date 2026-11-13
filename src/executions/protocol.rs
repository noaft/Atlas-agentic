use serde::{Deserialize, Serialize};
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ExecutionMode {
    Job,
    Task,
}
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutionPolicy {
    pub mode: ExecutionMode,
    #[serde(default = "default_yield")]
    pub yield_ms: u64,
}
fn default_yield() -> u64 {
    2000
}
impl ExecutionPolicy {
    pub fn validate(&self) -> Result<(), String> {
        super::validate_wait(self.yield_ms)
    }
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExecutionAction {
    Wait {
        ids: Vec<String>,
        yield_ms: u64,
    },
    Read {
        id: String,
        tail_lines: Option<usize>,
        cursor: Option<u64>,
    },
    List,
    Cancel {
        id: String,
    },
}
impl ExecutionAction {
    pub(crate) fn arguments(&self) -> serde_json::Value {
        match self {
            Self::Wait { ids, yield_ms } => {
                serde_json::json!({"execution_ids":ids,"yield_ms":yield_ms})
            }
            Self::Read {
                id,
                tail_lines,
                cursor,
            } => serde_json::json!({"execution_id":id,"tail_lines":tail_lines,"cursor":cursor}),
            Self::List => serde_json::json!({}),
            Self::Cancel { id } => serde_json::json!({"execution_id":id}),
        }
    }
    pub fn validate(&self) -> Result<(), String> {
        match self {
            Self::Wait { ids, yield_ms } => {
                super::validate_wait(*yield_ms)?;
                if ids.is_empty()
                    || ids.len() > 32
                    || ids.iter().any(|id| id.is_empty() || id.len() > 128)
                {
                    return Err("Wait for1–32 execution IDs".into());
                }
            }
            Self::Read {
                id,
                tail_lines,
                cursor,
            } => {
                valid_id(id)?;
                if tail_lines.is_some_and(|n| !(1..=200).contains(&n))
                    || tail_lines.is_some() && cursor.is_some()
                {
                    return Err("Read1–200 tail_lines or an absolute cursor, not both".into());
                }
            }
            Self::Cancel { id } => valid_id(id)?,
            Self::List => {}
        }
        Ok(())
    }
}
fn valid_id(id: &str) -> Result<(), String> {
    if id.is_empty() || id.len() > 128 {
        Err("Invalid execution ID".into())
    } else {
        Ok(())
    }
}
