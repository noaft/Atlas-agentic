use std::{
    io::Read,
    path::{Path, PathBuf},
};

use serde::Deserialize;

use super::{
    workspace_path::{open_workspace, validate_relative_path},
    Tool, ToolDefinition, ToolResult,
};

const DEFAULT_LIMIT: usize = 50;
const MAX_LIMIT: usize = 200;
const MAX_FILE_BYTES: u64 = 1024 * 1024;

pub struct SearchFilesTool {
    workspace: PathBuf,
}

impl SearchFilesTool {
    pub fn new(workspace: impl AsRef<Path>) -> Self {
        Self {
            workspace: workspace.as_ref().to_path_buf(),
        }
    }
}

impl Tool for SearchFilesTool {
    fn name(&self) -> &str {
        "search_files"
    }

    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: self.name().to_string(),
            description: "Search workspace file names and UTF-8 text contents.".to_string(),
            input_schema: serde_json::json!({
                "type":"object",
                "required":["query"],
                "properties":{
                    "query":{"type":"string"},
                    "root":{"type":"string","default":"."},
                    "glob":{"type":"string"},
                    "limit":{"type":"integer","minimum":1,"maximum":200,"default":50},
                    "includeHidden":{"type":"boolean","default":false}
                }
            }),
        }
    }

    fn execute(&self, args: &serde_json::Value) -> Result<ToolResult, String> {
        let args = parse(args)?;
        let workspace = open_workspace(self.name(), &self.workspace)?;
        let root_dir = workspace
            .open_dir(&args.root)
            .map_err(|error| format!("failed to open search_files root: {error}"))?;
        let mut matches = Vec::new();
        let mut stack = vec![(root_dir, args.root.clone())];

        while let Some((directory, relative_directory)) = stack.pop() {
            let mut entries = directory
                .entries()
                .map_err(|error| format!("failed to read search_files directory: {error}"))?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|error| format!("failed to read search_files entry: {error}"))?;
            entries.sort_by_key(|entry| entry.file_name());
            let mut child_directories = Vec::new();

            for entry in entries {
                let name = entry.file_name();
                let name_text = name.to_string_lossy();
                if !args.include_hidden && name_text.starts_with('.') {
                    continue;
                }
                let relative_path = if relative_directory == Path::new(".") {
                    PathBuf::from(&name)
                } else {
                    relative_directory.join(&name)
                };
                let file_type = entry
                    .file_type()
                    .map_err(|error| format!("failed to inspect search_files entry: {error}"))?;
                if file_type.is_symlink() {
                    continue;
                }
                if file_type.is_dir() {
                    if generated_directory(&name_text) {
                        continue;
                    }
                    let child = entry.open_dir().map_err(|error| {
                        format!("failed to open search_files directory: {error}")
                    })?;
                    child_directories.push((child, relative_path));
                    continue;
                }
                if !file_type.is_file()
                    || args
                        .glob
                        .as_deref()
                        .is_some_and(|glob| !wildcard_matches(glob, &path_text(&relative_path)))
                {
                    continue;
                }

                if name_text.contains(&args.query) {
                    matches.push(format!("filename {}", path_text(&relative_path)));
                    if matches.len() == args.limit {
                        return Ok(result(matches));
                    }
                }

                let mut bytes = Vec::new();
                entry
                    .open()
                    .and_then(|file| file.take(MAX_FILE_BYTES + 1).read_to_end(&mut bytes))
                    .map_err(|error| format!("failed to read search_files file: {error}"))?;
                if bytes.len() as u64 > MAX_FILE_BYTES || bytes.contains(&0) {
                    continue;
                }
                let Ok(content) = std::str::from_utf8(&bytes) else {
                    continue;
                };
                for (line_number, line) in content.lines().enumerate() {
                    if line.contains(&args.query) {
                        matches.push(format!(
                            "content {}:{}: {}",
                            path_text(&relative_path),
                            line_number + 1,
                            excerpt(line)
                        ));
                        if matches.len() == args.limit {
                            return Ok(result(matches));
                        }
                    }
                }
            }
            child_directories.reverse();
            stack.extend(child_directories);
        }

        Ok(result(matches))
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawArgs {
    query: String,
    root: Option<String>,
    glob: Option<String>,
    limit: Option<usize>,
    #[serde(default)]
    include_hidden: bool,
}

struct Args {
    query: String,
    root: PathBuf,
    glob: Option<String>,
    limit: usize,
    include_hidden: bool,
}

fn parse(args: &serde_json::Value) -> Result<Args, String> {
    let args: RawArgs = serde_json::from_value(args.clone())
        .map_err(|error| format!("invalid search_files args: {error}"))?;
    if args.query.is_empty() {
        return Err("search_files query is required".to_string());
    }
    let limit = args.limit.unwrap_or(DEFAULT_LIMIT);
    if !(1..=MAX_LIMIT).contains(&limit) {
        return Err(format!(
            "search_files limit must be between 1 and {MAX_LIMIT}"
        ));
    }
    let root = validate_relative_path("search_files", "root", args.root.as_deref().unwrap_or("."))?;
    Ok(Args {
        query: args.query,
        root,
        glob: args.glob.filter(|glob| !glob.is_empty()),
        limit,
        include_hidden: args.include_hidden,
    })
}

fn generated_directory(name: &str) -> bool {
    matches!(name, ".git" | "target" | "node_modules")
}

fn result(matches: Vec<String>) -> ToolResult {
    ToolResult {
        message: if matches.is_empty() {
            "No matches.".to_string()
        } else {
            matches.join("\n")
        },
    }
}

fn path_text(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

fn excerpt(line: &str) -> String {
    let line = line.trim();
    if line.chars().count() <= 160 {
        return line.to_string();
    }
    let mut value = line.chars().take(159).collect::<String>();
    value.push('…');
    value
}

fn wildcard_matches(pattern: &str, value: &str) -> bool {
    let pattern = pattern.as_bytes();
    let value = value.as_bytes();
    let (mut pattern_index, mut value_index) = (0, 0);
    let (mut star, mut retry) = (None, 0);
    while value_index < value.len() {
        if pattern_index < pattern.len()
            && (pattern[pattern_index] == b'?' || pattern[pattern_index] == value[value_index])
        {
            pattern_index += 1;
            value_index += 1;
        } else if pattern_index < pattern.len() && pattern[pattern_index] == b'*' {
            star = Some(pattern_index);
            pattern_index += 1;
            retry = value_index;
        } else if let Some(star_index) = star {
            pattern_index = star_index + 1;
            retry += 1;
            value_index = retry;
        } else {
            return false;
        }
    }
    while pattern_index < pattern.len() && pattern[pattern_index] == b'*' {
        pattern_index += 1;
    }
    pattern_index == pattern.len()
}
