use std::{
    any::Any,
    fs::{self, File},
    path::{Path, PathBuf},
};

use grep_regex::{RegexMatcher, RegexMatcherBuilder};
use grep_searcher::{sinks, BinaryDetection, SearcherBuilder};
use ignore::{
    overrides::{Override, OverrideBuilder},
    WalkBuilder,
};
use serde::Deserialize;

use crate::permissions::ToolRisk;

use super::{Tool, ToolDefinition, ToolResult};

const DEFAULT_LIMIT: usize = 50;
const MAX_LIMIT: usize = 200;
const MAX_LINE_CHARS: usize = 500;
const MAX_OUTPUT_BYTES: usize = 64 * 1024;
const TRUNCATION_NOTICE: &str = "[results truncated]";

pub struct RipgrepTool {
    workspace: PathBuf,
}

impl RipgrepTool {
    pub fn new(workspace: impl AsRef<Path>) -> Self {
        let workspace = workspace.as_ref();
        Self {
            workspace: fs::canonicalize(workspace).unwrap_or_else(|_| workspace.to_path_buf()),
        }
    }

    fn resolve_root(&self, path: Option<&str>) -> Result<PathBuf, String> {
        let requested = Path::new(path.unwrap_or("."));
        let candidate = if requested.is_absolute() {
            requested.to_path_buf()
        } else {
            self.workspace.join(requested)
        };
        let root = fs::canonicalize(&candidate).map_err(|error| {
            format!(
                "failed to resolve ripgrep path {}: {error}",
                candidate.display()
            )
        })?;
        validate_root_kind(&root)?;
        Ok(root)
    }
}

impl Tool for RipgrepTool {
    fn name(&self) -> &str {
        "ripgrep"
    }

    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: self.name().to_string(),
            description: "Search file contents with native ripgrep matching.".to_string(),
            input_schema: serde_json::json!({
                "type":"object",
                "additionalProperties":false,
                "required":["pattern"],
                "properties":{
                    "pattern":{"type":"string"},
                    "path":{"type":"string","default":"."},
                    "glob":{"type":"string"},
                    "fixedStrings":{"type":"boolean","default":false},
                    "caseSensitive":{"type":"boolean","default":true},
                    "includeHidden":{"type":"boolean","default":false},
                    "includeIgnored":{"type":"boolean","default":false},
                    "limit":{"type":"integer","minimum":1,"maximum":200,"default":50}
                }
            }),
        }
    }

    fn risk(&self, args: &serde_json::Value) -> Result<ToolRisk, String> {
        self.authorize(args).map(|(risk, _)| risk)
    }

    fn authorize(
        &self,
        args: &serde_json::Value,
    ) -> Result<(ToolRisk, Option<Box<dyn Any>>), String> {
        let args = parse(args)?;
        let root = self.resolve_root(args.path.as_deref())?;
        build_matcher(&args)?;
        build_overrides(&root, args.glob.as_deref())?;
        let risk = if root == self.workspace || root.starts_with(&self.workspace) {
            ToolRisk::ReadOnly
        } else {
            ToolRisk::ReadOutsideWorkspace
        };
        Ok((risk, Some(Box::new(root))))
    }

    fn execute(&self, args: &serde_json::Value) -> Result<ToolResult, String> {
        let args = parse(args)?;
        let root = self.resolve_root(args.path.as_deref())?;
        self.execute_resolved(args, root)
    }

    fn execute_authorized(
        &self,
        args: &serde_json::Value,
        context: Option<&dyn Any>,
    ) -> Result<ToolResult, String> {
        let args = parse(args)?;
        let root = self.resolve_root(args.path.as_deref())?;
        let authorized_root = context
            .and_then(|context| context.downcast_ref::<PathBuf>())
            .ok_or_else(|| "ripgrep authorization context is missing".to_string())?;
        if &root != authorized_root {
            return Err("ripgrep path changed after authorization".to_string());
        }
        self.execute_resolved(args, root)
    }
}

impl RipgrepTool {
    fn execute_resolved(&self, args: Args, root: PathBuf) -> Result<ToolResult, String> {
        ensure_root_readable(&root)?;
        let matcher = build_matcher(&args)?;
        let overrides = build_overrides(&root, args.glob.as_deref())?;

        let mut walker = WalkBuilder::new(&root);
        walker
            .hidden(!args.include_hidden)
            .follow_links(false)
            .sort_by_file_name(|left, right| left.cmp(right));
        if args.include_ignored {
            walker
                .ignore(false)
                .git_global(false)
                .git_ignore(false)
                .git_exclude(false)
                .parents(false);
        } else {
            walker.add_custom_ignore_filename(".rgignore");
        }
        if let Some(overrides) = overrides {
            walker.overrides(overrides);
        }

        let mut output = SearchOutput::new(args.limit);
        let mut warnings = 0usize;
        for entry in walker.build() {
            let entry = match entry {
                Ok(entry) => entry,
                Err(_) => {
                    warnings = warnings.saturating_add(1);
                    continue;
                }
            };
            if !entry.file_type().is_some_and(|kind| kind.is_file()) {
                continue;
            }
            let label = display_path(entry.path(), &self.workspace);
            let mut searcher = SearcherBuilder::new()
                .line_number(true)
                .binary_detection(BinaryDetection::quit(b'\0'))
                .build();
            let search = searcher
                .search_path(
                    &matcher,
                    entry.path(),
                    sinks::Bytes(|line_number, bytes| {
                        Ok(output.push(format!("{label}:{line_number}:{}", excerpt(bytes))))
                    }),
                )
                .map_err(|error| format!("failed to search {}: {error}", entry.path().display()));
            if let Err(error) = search {
                if entry.path() == root {
                    return Err(error);
                }
                warnings = warnings.saturating_add(1);
            }
            if output.truncated {
                break;
            }
        }

        Ok(ToolResult::text(output.finish(warnings)))
    }
}

fn validate_root_kind(root: &Path) -> Result<(), String> {
    let metadata = fs::metadata(root)
        .map_err(|error| format!("failed to inspect ripgrep path {}: {error}", root.display()))?;
    if metadata.is_file() || metadata.is_dir() {
        Ok(())
    } else {
        Err(format!(
            "ripgrep path must be a regular file or directory: {}",
            root.display()
        ))
    }
}

fn ensure_root_readable(root: &Path) -> Result<(), String> {
    if root.is_file() {
        File::open(root)
            .map(|_| ())
            .map_err(|error| format!("failed to open ripgrep path {}: {error}", root.display()))
    } else {
        fs::read_dir(root)
            .map(|_| ())
            .map_err(|error| format!("failed to open ripgrep path {}: {error}", root.display()))
    }
}

fn build_matcher(args: &Args) -> Result<RegexMatcher, String> {
    RegexMatcherBuilder::new()
        .case_insensitive(!args.case_sensitive)
        .fixed_strings(args.fixed_strings)
        .build(&args.pattern)
        .map_err(|error| format!("invalid ripgrep pattern: {error}"))
}

fn build_overrides(root: &Path, glob: Option<&str>) -> Result<Option<Override>, String> {
    let Some(glob) = glob else {
        return Ok(None);
    };
    let mut overrides = OverrideBuilder::new(root);
    overrides
        .add(glob)
        .map_err(|error| format!("invalid ripgrep glob: {error}"))?;
    overrides
        .build()
        .map(Some)
        .map_err(|error| format!("invalid ripgrep glob: {error}"))
}

struct SearchOutput {
    lines: Vec<String>,
    bytes: usize,
    limit: usize,
    truncated: bool,
}

impl SearchOutput {
    fn new(limit: usize) -> Self {
        Self {
            lines: Vec::new(),
            bytes: 0,
            limit,
            truncated: false,
        }
    }

    fn push(&mut self, line: String) -> bool {
        let separator = usize::from(!self.lines.is_empty());
        let notice_bytes = usize::from(!self.lines.is_empty()) + TRUNCATION_NOTICE.len();
        if self.bytes + separator + line.len() + notice_bytes > MAX_OUTPUT_BYTES {
            self.truncated = true;
            return false;
        }
        self.bytes += separator + line.len();
        self.lines.push(line);
        if self.lines.len() >= self.limit {
            self.truncated = true;
            return false;
        }
        true
    }

    fn finish(self, warnings: usize) -> String {
        let mut result = if self.lines.is_empty() {
            "No matches.".to_string()
        } else {
            self.lines.join("\n")
        };
        if self.truncated {
            result.push('\n');
            result.push_str(TRUNCATION_NOTICE);
        }
        if warnings > 0 {
            let warning = format!("\n[skipped {warnings} unreadable paths]");
            if result.len() + warning.len() <= MAX_OUTPUT_BYTES {
                result.push_str(&warning);
            }
        }
        result
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RawArgs {
    pattern: String,
    path: Option<String>,
    glob: Option<String>,
    #[serde(default)]
    fixed_strings: bool,
    #[serde(default = "default_true")]
    case_sensitive: bool,
    #[serde(default)]
    include_hidden: bool,
    #[serde(default)]
    include_ignored: bool,
    limit: Option<usize>,
}

struct Args {
    pattern: String,
    path: Option<String>,
    glob: Option<String>,
    fixed_strings: bool,
    case_sensitive: bool,
    include_hidden: bool,
    include_ignored: bool,
    limit: usize,
}

fn parse(args: &serde_json::Value) -> Result<Args, String> {
    let args: RawArgs = serde_json::from_value(args.clone())
        .map_err(|error| format!("invalid ripgrep args: {error}"))?;
    if args.pattern.is_empty() {
        return Err("ripgrep pattern is required".to_string());
    }
    let limit = args.limit.unwrap_or(DEFAULT_LIMIT);
    if !(1..=MAX_LIMIT).contains(&limit) {
        return Err(format!("ripgrep limit must be between 1 and {MAX_LIMIT}"));
    }
    Ok(Args {
        pattern: args.pattern,
        path: args.path,
        glob: args.glob.filter(|glob| !glob.is_empty()),
        fixed_strings: args.fixed_strings,
        case_sensitive: args.case_sensitive,
        include_hidden: args.include_hidden,
        include_ignored: args.include_ignored,
        limit,
    })
}

fn default_true() -> bool {
    true
}

fn display_path(path: &Path, workspace: &Path) -> String {
    path.strip_prefix(workspace)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}

fn excerpt(bytes: &[u8]) -> String {
    let text = String::from_utf8_lossy(bytes);
    let text = text.trim_end_matches(['\r', '\n']);
    if text.chars().count() <= MAX_LINE_CHARS {
        return text.to_string();
    }
    let mut value = text.chars().take(MAX_LINE_CHARS - 1).collect::<String>();
    value.push('…');
    value
}
