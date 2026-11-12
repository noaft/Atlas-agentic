use std::{
    collections::BinaryHeap,
    ffi::OsString,
    path::{Path, PathBuf},
};

use cap_std::fs::Dir;
use serde::{Deserialize, Serialize};

use super::{
    inspection::{page_message, OutputFormat, MAX_OUTPUT_BYTES},
    workspace_path::{open_workspace, validate_relative_path},
    Tool, ToolDefinition, ToolResult,
};

const DEFAULT_LIMIT: usize = 100;
const MAX_LIMIT: usize = 1000;
const MAX_OFFSET: usize = 100_000;

pub struct ListDirTool {
    workspace: PathBuf,
}

impl ListDirTool {
    pub fn new(workspace: impl AsRef<Path>) -> Self {
        Self {
            workspace: workspace.as_ref().to_path_buf(),
        }
    }
}

impl Tool for ListDirTool {
    fn name(&self) -> &str {
        "ls"
    }

    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: self.name().to_string(),
            description: "List a workspace directory alphabetically with bounded pagination (100 entries by default, 64 KiB output). Prefer format=json for file/directory/symlink kinds and next_cursor; merge the cursor with path to continue. Pages reflect the current directory, not a frozen snapshot.".to_string(),
            input_schema: serde_json::json!({
                "type":"object", "additionalProperties":false,
                "properties": {
                    "path":{"type":"string","description":"Relative workspace directory; defaults to the workspace root."},
                    "offset":{"type":"integer","minimum":0,"maximum":MAX_OFFSET,"default":0},
                    "limit":{"type":"integer","minimum":1,"maximum":MAX_LIMIT,"default":DEFAULT_LIMIT},
                    "format":{"type":"string","enum":["text","json"],"default":"text"}
                }
            }),
        }
    }

    fn risk(&self, args: &serde_json::Value) -> Result<crate::permissions::ToolRisk, String> {
        parse(args)?;
        Ok(crate::permissions::ToolRisk::ReadOnly)
    }

    fn execute(&self, args: &serde_json::Value) -> Result<ToolResult, String> {
        let args = parse(args)?;
        let relative_path = validate_relative_path("ls", "path", &args.path)?;
        let workspace = open_workspace(self.name(), &self.workspace)?;
        let directory = workspace
            .open_dir(&relative_path)
            .map_err(|error| format!("failed to open ls directory: {error}"))?;
        let names = page_names(&directory, args.offset, args.limit)?;
        let entries = names
            .iter()
            .map(|name| {
                let metadata = directory
                    .symlink_metadata(name)
                    .map_err(|error| format!("failed to inspect ls entry: {error}"))?;
                let kind = if metadata.is_symlink() {
                    "symlink"
                } else if metadata.is_dir() {
                    "directory"
                } else if metadata.is_file() {
                    "file"
                } else {
                    "other"
                };
                let path = if relative_path == Path::new(".") {
                    PathBuf::from(name)
                } else {
                    relative_path.join(name)
                };
                Ok(DirectoryEntry {
                    path: path.display().to_string(),
                    kind,
                })
            })
            .collect::<Result<Vec<_>, String>>()?;
        Ok(ToolResult::text(bounded_page(&args, &entries)?))
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ListDirArgs {
    #[serde(default = "default_path")]
    path: String,
    #[serde(default)]
    offset: usize,
    #[serde(default = "default_limit")]
    limit: usize,
    #[serde(default)]
    format: OutputFormat,
}

fn default_path() -> String {
    ".".into()
}

fn default_limit() -> usize {
    DEFAULT_LIMIT
}

fn parse(value: &serde_json::Value) -> Result<ListDirArgs, String> {
    let args: ListDirArgs = serde_json::from_value(value.clone())
        .map_err(|error| format!("invalid ls args: {error}"))?;
    validate_relative_path("ls", "path", &args.path)?;
    if !(1..=MAX_LIMIT).contains(&args.limit) || args.offset > MAX_OFFSET {
        return Err(format!(
            "ls limit must be 1..={MAX_LIMIT} and offset 0..={MAX_OFFSET}"
        ));
    }
    Ok(args)
}

fn page_names(directory: &Dir, offset: usize, limit: usize) -> Result<Vec<OsString>, String> {
    // Only retain the smallest offset+limit+1 names rather than the whole directory.
    let retained = offset + limit + 1;
    let mut names = BinaryHeap::new();
    for entry in directory
        .entries()
        .map_err(|error| format!("failed to list directory: {error}"))?
    {
        let name = entry
            .map_err(|error| format!("failed to read directory entry: {error}"))?
            .file_name();
        if names.len() < retained {
            names.push(name);
        } else if names.peek().is_some_and(|last| &name < last) {
            names.pop();
            names.push(name);
        }
    }
    Ok(names.into_sorted_vec().into_iter().skip(offset).collect())
}

#[derive(Serialize)]
struct DirectoryEntry {
    path: String,
    kind: &'static str,
}

fn render_page(
    args: &ListDirArgs,
    entries: &[DirectoryEntry],
    count: usize,
) -> Result<String, String> {
    let truncated = count < entries.len();
    let next_offset = args.offset + count;
    // Do not advertise a cursor that the next call would reject.
    let can_continue = next_offset <= MAX_OFFSET;
    if matches!(args.format, OutputFormat::Text) {
        let mut text = entries[..count]
            .iter()
            .map(|entry| entry.path.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        if truncated {
            if can_continue {
                text.push_str(&format!(
                    "\n[ls output truncated; next offset={next_offset}, limit={}]",
                    args.limit
                ));
            } else {
                text.push_str("\n[ls output truncated; pagination limit reached; use a narrower directory or search]");
            }
        }
        return Ok(text);
    }
    #[derive(Serialize)]
    struct DirectoryData<'a> {
        path: &'a str,
        entries: &'a [DirectoryEntry],
        offset: usize,
        limit: usize,
        pagination_limit_reached: bool,
    }
    page_message(
        DirectoryData {
            path: &args.path,
            entries: &entries[..count],
            offset: args.offset,
            limit: args.limit,
            pagination_limit_reached: truncated && !can_continue,
        },
        truncated,
        (truncated && can_continue)
            .then(|| serde_json::json!({"offset":next_offset,"limit":args.limit,"format":"json"})),
    )
}

fn bounded_page(args: &ListDirArgs, entries: &[DirectoryEntry]) -> Result<String, String> {
    let maximum = entries.len().min(args.limit);
    let complete = render_page(args, entries, maximum)?;
    if complete.len() <= MAX_OUTPUT_BYTES {
        return Ok(complete);
    }
    let mut low = 0;
    let mut high = maximum;
    while low + 1 < high {
        let middle = (low + high) / 2;
        if render_page(args, entries, middle)?.len() <= MAX_OUTPUT_BYTES {
            low = middle;
        } else {
            high = middle;
        }
    }
    if low == 0 {
        return Err(
            "ls entry or directory path is too large to return within the output limit".into(),
        );
    }
    render_page(args, entries, low)
}
