use std::{
    io::{BufRead, BufReader, Seek, SeekFrom},
    path::{Path, PathBuf},
};

use cap_std::fs::OpenOptions;
#[cfg(unix)]
use cap_std::fs::OpenOptionsExt;
use serde::{Deserialize, Serialize};

use super::{
    inspection::{page_message, OutputFormat, MAX_OUTPUT_BYTES},
    workspace_path::{open_workspace, validate_relative_path},
    Tool, ToolDefinition, ToolResult,
};

const MAX_SCAN_BYTES: usize = 32 * 1024 * 1024;
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
            description: "Read workspace UTF-8 text, optionally by inclusive 1-based line range with line numbers. Output is capped at 64 KiB. Prefer format=json for metadata and next_cursor; merge that cursor with path to continue long files/lines. Byte cursors return unnumbered text. Line-start scans are capped at 32 MiB.".to_string(),
            input_schema: serde_json::json!({
                "type":"object", "required":["path"], "additionalProperties":false,
                "properties": {
                    "path":{"type":"string","description":"Relative workspace file path."},
                    "start_line":{"type":"integer","minimum":1,"description":"First line, inclusive; defaults to 1."},
                    "end_line":{"type":"integer","minimum":1,"description":"Last line, inclusive; omitted means read onward."},
                    "line_numbers":{"type":"boolean","default":false},
                    "byte_offset":{"type":"integer","minimum":0,"description":"UTF-8 boundary for continuation; cannot combine with line range/numbering."},
                    "line_count":{"type":"integer","minimum":1,"description":"Optional line cap; next_cursor preserves remaining lines of a range. Cannot combine with end_line."},
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
        let workspace = open_workspace(self.name(), &self.workspace)?;
        let mut options = OpenOptions::new();
        options.read(true);
        #[cfg(unix)]
        options.custom_flags(libc::O_NONBLOCK);
        let mut file = workspace
            .open_with(&args.path, &options)
            .map_err(|error| format!("failed to open read_file target: {error}"))?;
        if !file
            .metadata()
            .map_err(|error| format!("failed to inspect read_file target: {error}"))?
            .is_file()
        {
            return Err(format!("read_file target is not a file: {}", args.path));
        }
        if let Some(offset) = args.byte_offset {
            file.seek(SeekFrom::Start(offset))
                .map_err(|error| format!("failed to seek read_file target: {error}"))?;
        }
        let mut reader = BufReader::new(file);
        let start_byte = match args.byte_offset {
            Some(offset) => offset,
            None => skip_lines(&mut reader, args.start_line.unwrap_or(1) - 1)?,
        };
        let bytes = read_selection(&mut reader, args.selected_lines())?;
        let text = decode_selection(&bytes)?;
        let message = bounded_output(&args, text, start_byte, bytes.len())?;
        Ok(ToolResult::text(message))
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ReadFileArgs {
    path: String,
    start_line: Option<u64>,
    end_line: Option<u64>,
    byte_offset: Option<u64>,
    line_count: Option<u64>,
    #[serde(default)]
    line_numbers: bool,
    #[serde(default)]
    format: OutputFormat,
}

impl ReadFileArgs {
    fn selected_lines(&self) -> Option<u64> {
        self.line_count.or_else(|| {
            self.end_line
                .map(|end| end - self.start_line.unwrap_or(1) + 1)
        })
    }
}

fn parse(value: &serde_json::Value) -> Result<ReadFileArgs, String> {
    let args: ReadFileArgs = serde_json::from_value(value.clone())
        .map_err(|error| format!("invalid read_file args: {error}"))?;
    validate_relative_path("read_file", "path", &args.path)?;
    if args.start_line == Some(0)
        || args
            .end_line
            .is_some_and(|end| end < args.start_line.unwrap_or(1))
        || args.line_count == Some(0)
    {
        return Err("read_file requires positive lines and end_line >= start_line".into());
    }
    if args.line_count.is_some() && args.end_line.is_some() {
        return Err("read_file line_count cannot combine with end_line".into());
    }
    if args.byte_offset.is_some()
        && (args.start_line.is_some() || args.end_line.is_some() || args.line_numbers)
    {
        return Err("read_file byte_offset cannot combine with line range or numbering".into());
    }
    Ok(args)
}

fn skip_lines(reader: &mut impl BufRead, mut remaining: u64) -> Result<u64, String> {
    let mut scanned = 0;
    while remaining > 0 {
        let available = reader
            .fill_buf()
            .map_err(|error| format!("failed to read file: {error}"))?;
        if available.is_empty() {
            break;
        }
        if scanned == MAX_SCAN_BYTES {
            return Err(
                "read_file line scan exceeds 32 MiB; use byte_offset to inspect later bytes".into(),
            );
        }
        let available = &available[..available.len().min(MAX_SCAN_BYTES - scanned)];
        let mut consumed = available.len();
        for (index, byte) in available.iter().enumerate() {
            if *byte == b'\n' {
                remaining -= 1;
                if remaining == 0 {
                    consumed = index + 1;
                    break;
                }
            }
        }
        reader.consume(consumed);
        scanned += consumed;
    }
    Ok(scanned as u64)
}

fn read_selection(reader: &mut impl BufRead, mut lines: Option<u64>) -> Result<Vec<u8>, String> {
    let mut bytes = Vec::new();
    while bytes.len() <= MAX_OUTPUT_BYTES && lines != Some(0) {
        let available = reader
            .fill_buf()
            .map_err(|error| format!("failed to read file: {error}"))?;
        if available.is_empty() {
            break;
        }
        let available = &available[..available.len().min(MAX_OUTPUT_BYTES + 1 - bytes.len())];
        let mut consumed = available.len();
        if let Some(remaining) = lines.as_mut() {
            for (index, byte) in available.iter().enumerate() {
                if *byte == b'\n' {
                    *remaining -= 1;
                    if *remaining == 0 {
                        consumed = index + 1;
                        break;
                    }
                }
            }
        }
        bytes.extend_from_slice(&available[..consumed]);
        reader.consume(consumed);
    }
    Ok(bytes)
}

fn decode_selection(bytes: &[u8]) -> Result<&str, String> {
    let bounded = bytes.len() > MAX_OUTPUT_BYTES;
    let bytes = &bytes[..bytes.len().min(MAX_OUTPUT_BYTES)];
    match std::str::from_utf8(bytes) {
        Ok(text) => Ok(text),
        Err(error) if bounded && error.error_len().is_none() => {
            Ok(std::str::from_utf8(&bytes[..error.valid_up_to()]).unwrap())
        }
        Err(_) => Err("read_file only supports UTF-8 text files".into()),
    }
}

fn render_output(
    args: &ReadFileArgs,
    text: &str,
    start_byte: u64,
    truncated: bool,
) -> Result<String, String> {
    let content = if args.line_numbers {
        text.split_inclusive('\n')
            .enumerate()
            .map(|(index, line)| format!("{}: {line}", args.start_line.unwrap_or(1) + index as u64))
            .collect::<String>()
    } else {
        text.to_string()
    };
    if matches!(args.format, OutputFormat::Text) {
        return Ok(if truncated {
            format!("{content}{TRUNCATION_NOTICE}")
        } else {
            content
        });
    }
    #[derive(Serialize)]
    struct FileData<'a> {
        path: &'a str,
        content: String,
        start_byte: u64,
        end_byte: u64,
        start_line: Option<u64>,
    }
    let end_byte = start_byte + text.len() as u64;
    let next = truncated.then(|| {
        let mut cursor = serde_json::json!({"byte_offset":end_byte,"format":"json"});
        if let Some(lines) = args.selected_lines() {
            let consumed = text.bytes().filter(|byte| *byte == b'\n').count() as u64;
            cursor["line_count"] = (lines - consumed).into();
        }
        cursor
    });
    page_message(
        FileData {
            path: &args.path,
            content,
            start_byte,
            end_byte,
            start_line: args
                .byte_offset
                .is_none()
                .then(|| args.start_line.unwrap_or(1)),
        },
        truncated,
        next,
    )
}

fn bounded_output(
    args: &ReadFileArgs,
    text: &str,
    start_byte: u64,
    source_bytes: usize,
) -> Result<String, String> {
    let complete = render_output(args, text, start_byte, text.len() < source_bytes)?;
    if complete.len() <= MAX_OUTPUT_BYTES {
        return Ok(complete);
    }
    // Search UTF-8 boundaries, accounting for numbering, JSON escapes and cursor overhead.
    let boundaries: Vec<_> = text.char_indices().map(|(index, _)| index).collect();
    let mut low = 0;
    let mut high = boundaries.len();
    while low + 1 < high {
        let middle = (low + high) / 2;
        let output = render_output(args, &text[..boundaries[middle]], start_byte, true)?;
        if output.len() <= MAX_OUTPUT_BYTES {
            low = middle;
        } else {
            high = middle;
        }
    }
    render_output(args, &text[..boundaries[low]], start_byte, true)
}
