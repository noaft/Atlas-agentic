use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum WakeupMode {
    Hard,
    Lazy,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ExecutionPolicy {
    pub mode: WakeupMode,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub interval_seconds: Option<f64>,
}

impl<'de> Deserialize<'de> for ExecutionPolicy {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(tag = "mode", rename_all = "lowercase", deny_unknown_fields)]
        enum Policy {
            Hard { interval_seconds: f64 },
            Lazy {},
        }
        let policy = match Policy::deserialize(deserializer)? {
            Policy::Hard { interval_seconds } => Self {
                mode: WakeupMode::Hard,
                interval_seconds: Some(interval_seconds),
            },
            Policy::Lazy {} => Self {
                mode: WakeupMode::Lazy,
                interval_seconds: None,
            },
        };
        policy.validate().map_err(serde::de::Error::custom)?;
        Ok(policy)
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ExecutionDefaults {
    default_mode: WakeupMode,
    hard_interval_seconds: f64,
    lazy_interval_seconds: f64,
    #[serde(rename = "persistence_max_records")]
    _persistence_max_records: usize,
    #[serde(rename = "persistence_max_bytes")]
    _persistence_max_bytes: u64,
}

fn defaults() -> &'static ExecutionDefaults {
    static DEFAULTS: std::sync::OnceLock<ExecutionDefaults> = std::sync::OnceLock::new();
    DEFAULTS.get_or_init(|| {
        toml::from_str(include_str!("../../config/execution-defaults.toml"))
            .expect("bundled execution defaults must be valid")
    })
}

impl Default for ExecutionPolicy {
    fn default() -> Self {
        let config = defaults();
        Self {
            mode: config.default_mode,
            interval_seconds: (config.default_mode == WakeupMode::Hard)
                .then_some(config.hard_interval_seconds),
        }
    }
}

impl ExecutionPolicy {
    pub fn validate(&self) -> Result<(), String> {
        self.interval_ms().map(|_| ())
    }

    pub fn interval_ms(&self) -> Result<u64, String> {
        match (self.mode, self.interval_seconds) {
            (WakeupMode::Hard, Some(seconds))
                if seconds.is_finite() && (0.1..30.0).contains(&seconds) =>
            {
                Ok((seconds * 1000.0).round().clamp(100.0, 29999.0) as u64)
            }
            (WakeupMode::Lazy, None) => Ok((defaults().lazy_interval_seconds * 1000.0) as u64),
            (WakeupMode::Hard, _) => {
                Err("hard wake_up requires finite interval_seconds from 0.1 to less than 30".into())
            }
            (WakeupMode::Lazy, Some(_)) => Err("lazy wake_up forbids interval_seconds".into()),
        }
    }
}

/// Render model instructions from the same bundled defaults used by scheduling.
pub fn wakeup_prompt() -> String {
    let config = defaults();
    include_str!("../../config/wakeup-prompt.md")
        .replace(
            "{{default_wakeup}}",
            &serde_json::to_string(&ExecutionPolicy::default())
                .expect("valid default wakeup policy"),
        )
        .replace(
            "{{hard_interval_seconds}}",
            &config.hard_interval_seconds.to_string(),
        )
        .replace(
            "{{lazy_interval_seconds}}",
            &config.lazy_interval_seconds.to_string(),
        )
}

#[derive(Debug, Clone, PartialEq)]
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
    SetWakeup {
        id: String,
        wake_up: ExecutionPolicy,
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
            Self::SetWakeup { id, wake_up } => {
                serde_json::json!({"execution_id":id,"wake_up":wake_up})
            }
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
            Self::SetWakeup { id, wake_up } => {
                valid_id(id)?;
                wake_up.validate()?;
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
