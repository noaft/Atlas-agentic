use std::{
    io::Read,
    path::{Component, Path, PathBuf},
};

#[cfg(unix)]
use cap_std::fs::OpenOptionsExt;
use cap_std::{
    ambient_authority,
    fs::{Dir, OpenOptions},
};
use serde::Deserialize;

use super::{Tool, ToolDefinition, ToolResult};

const MAX_OUTPUT_BYTES: usize = 64 * 1024;
const TRUNCATION_NOTICE: &str = "\n[read_file output truncated]";

pub struct ReadFileTool {
    workspace: PathBuf,
}

impl ReadFileTool {
    pub fn new(workspace: impl AsRef<Path>) -> Self {
        Self {
            workspace: workspace.as_ref().to_path_buf(),
        }
    }
}

impl Tool for ReadFileTool {
    fn name(&self) -> &str {
        "read_file"
    }

    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: self.name().to_string(),
            description: "Read a UTF-8 text file inside the workspace.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "required": ["path"],
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "Relative file path inside the workspace."
                    }
                }
            }),
        }
    }

    fn execute(&self, args: &serde_json::Value) -> Result<ToolResult, String> {
        let args: ReadFileArgs = serde_json::from_value(args.clone())
            .map_err(|error| format!("invalid read_file args: {error}"))?;
        validate_relative_path(&args.path)?;

        let workspace = Dir::open_ambient_dir(&self.workspace, ambient_authority())
            .map_err(|error| format!("failed to open read_file workspace: {error}"))?;
        let mut options = OpenOptions::new();
        options.read(true);
        #[cfg(unix)]
        options.custom_flags(libc::O_NONBLOCK);
        let file = workspace
            .open_with(&args.path, &options)
            .map_err(|error| format!("failed to open read_file target: {error}"))?;
        if !file
            .metadata()
            .map_err(|error| format!("failed to inspect read_file target: {error}"))?
            .is_file()
        {
            return Err(format!("read_file target is not a file: {}", args.path));
        }

        let mut bytes = Vec::with_capacity(MAX_OUTPUT_BYTES + 1);
        file.take((MAX_OUTPUT_BYTES + 1) as u64)
            .read_to_end(&mut bytes)
            .map_err(|error| format!("failed to read file: {error}"))?;

        let message = decode_output(&bytes)?;
        Ok(ToolResult { message })
    }
}

#[derive(Deserialize)]
struct ReadFileArgs {
    path: String,
}

fn validate_relative_path(path: &str) -> Result<(), String> {
    let path = Path::new(path);
    if path.as_os_str().is_empty() {
        return Err("read_file path is required".to_string());
    }
    if path.is_absolute() {
        return Err("read_file path must be relative".to_string());
    }
    if path
        .components()
        .any(|component| matches!(component, Component::ParentDir))
    {
        return Err("read_file path cannot contain parent directory segments".to_string());
    }
    Ok(())
}

fn decode_output(bytes: &[u8]) -> Result<String, String> {
    if bytes.len() <= MAX_OUTPUT_BYTES {
        return std::str::from_utf8(bytes)
            .map(str::to_owned)
            .map_err(|_| "read_file only supports UTF-8 text files".to_string());
    }

    let content_limit = MAX_OUTPUT_BYTES - TRUNCATION_NOTICE.len();
    let prefix = utf8_prefix(&bytes[..content_limit])?;
    Ok(format!("{prefix}{TRUNCATION_NOTICE}"))
}

fn utf8_prefix(bytes: &[u8]) -> Result<&str, String> {
    match std::str::from_utf8(bytes) {
        Ok(content) => Ok(content),
        Err(error) if error.error_len().is_none() => {
            std::str::from_utf8(&bytes[..error.valid_up_to()])
                .map_err(|_| "read_file only supports UTF-8 text files".to_string())
        }
        Err(_) => Err("read_file only supports UTF-8 text files".to_string()),
    }
}
