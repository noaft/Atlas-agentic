use std::fs;

use atlas_agentic::tools::{ReadFileTool, Tool};
use tempfile::TempDir;

const MAX_OUTPUT_BYTES: usize = 64 * 1024;
const TRUNCATION_NOTICE: &str = "\n[read_file output truncated]";

fn execute(tool: &ReadFileTool, path: &str) -> Result<String, String> {
    tool.execute(&serde_json::json!({ "path": path }))
        .map(|result| result.message)
}

#[test]
fn definition_requires_a_relative_path() {
    let workspace = TempDir::new().unwrap();
    let definition = ReadFileTool::new(workspace.path()).definition();

    assert_eq!(definition.name, "read_file");
    assert_eq!(
        definition.input_schema["required"],
        serde_json::json!(["path"])
    );
    assert_eq!(
        definition.input_schema["properties"]["path"]["type"],
        "string"
    );
}

#[test]
fn reads_utf8_text_from_a_nested_workspace_file() {
    let workspace = TempDir::new().unwrap();
    fs::create_dir(workspace.path().join("src")).unwrap();
    fs::write(
        workspace.path().join("src/lib.rs"),
        "pub fn greeting() {\n    println!(\"hello 🌍\");\n}\n",
    )
    .unwrap();

    assert_eq!(
        execute(&ReadFileTool::new(workspace.path()), "src/lib.rs").unwrap(),
        "pub fn greeting() {\n    println!(\"hello 🌍\");\n}\n"
    );
}

#[test]
fn rejects_paths_that_can_leave_or_do_not_name_a_file() {
    let workspace = TempDir::new().unwrap();
    fs::create_dir(workspace.path().join("src")).unwrap();

    for (path, expected) in [
        ("", "read_file path is required"),
        (
            "../secret.txt",
            "read_file path cannot contain parent directory segments",
        ),
        ("/etc/passwd", "read_file path must be relative"),
        ("src", "read_file target is not a file: src"),
        ("missing.txt", "failed to open read_file target"),
    ] {
        let error = execute(&ReadFileTool::new(workspace.path()), path).unwrap_err();
        assert!(
            error.contains(expected),
            "path {path:?}: expected {expected:?}, got {error:?}"
        );
    }
}

#[test]
fn rejects_non_utf8_content() {
    let workspace = TempDir::new().unwrap();
    fs::write(workspace.path().join("binary.dat"), [0xff, 0xfe, 0xfd]).unwrap();

    assert_eq!(
        execute(&ReadFileTool::new(workspace.path()), "binary.dat").unwrap_err(),
        "read_file only supports UTF-8 text files"
    );
}

#[test]
fn truncates_large_output_without_splitting_utf8() {
    let workspace = TempDir::new().unwrap();
    let mut content = "a".repeat(MAX_OUTPUT_BYTES - TRUNCATION_NOTICE.len() - 1);
    content.push('🌍');
    content.push_str(&"b".repeat(64));
    fs::write(workspace.path().join("large.txt"), content).unwrap();

    let output = execute(&ReadFileTool::new(workspace.path()), "large.txt").unwrap();

    assert!(output.len() <= MAX_OUTPUT_BYTES);
    assert!(output.ends_with("\n[read_file output truncated]"));
    assert!(std::str::from_utf8(output.as_bytes()).is_ok());
}

#[test]
fn reads_empty_and_exact_limit_files_without_a_truncation_notice() {
    let workspace = TempDir::new().unwrap();
    fs::write(workspace.path().join("empty.txt"), "").unwrap();
    fs::write(
        workspace.path().join("exact.txt"),
        "a".repeat(MAX_OUTPUT_BYTES),
    )
    .unwrap();

    assert_eq!(
        execute(&ReadFileTool::new(workspace.path()), "empty.txt").unwrap(),
        ""
    );
    let exact = execute(&ReadFileTool::new(workspace.path()), "exact.txt").unwrap();
    assert_eq!(exact.len(), MAX_OUTPUT_BYTES);
    assert!(!exact.ends_with(TRUNCATION_NOTICE));
}

#[cfg(unix)]
#[test]
fn rejects_a_symlink_that_escapes_the_workspace() {
    use std::os::unix::fs::symlink;

    let workspace = TempDir::new().unwrap();
    let outside = TempDir::new().unwrap();
    let secret = outside.path().join("secret.txt");
    fs::write(&secret, "outside").unwrap();
    symlink(&secret, workspace.path().join("secret-link.txt")).unwrap();

    assert!(
        execute(&ReadFileTool::new(workspace.path()), "secret-link.txt")
            .unwrap_err()
            .contains("failed to open read_file target")
    );
}

#[cfg(unix)]
#[test]
fn rejects_a_fifo_without_blocking() {
    use std::{ffi::CString, os::unix::ffi::OsStrExt, time::Instant};

    let workspace = TempDir::new().unwrap();
    let fifo = workspace.path().join("pipe");
    let fifo_path = CString::new(fifo.as_os_str().as_bytes()).unwrap();
    assert_eq!(unsafe { libc::mkfifo(fifo_path.as_ptr(), 0o600) }, 0);

    let started = Instant::now();
    let error = execute(&ReadFileTool::new(workspace.path()), "pipe").unwrap_err();

    assert!(error.contains("read_file target is not a file"), "{error}");
    assert!(started.elapsed().as_secs() < 1);
}

#[cfg(target_os = "linux")]
#[test]
fn never_reads_outside_during_an_atomic_file_symlink_swap() {
    use std::{
        ffi::CString,
        os::unix::{ffi::OsStrExt, fs::symlink},
        sync::{
            atomic::{AtomicBool, Ordering},
            Arc,
        },
        thread,
    };

    let workspace = TempDir::new().unwrap();
    let outside = TempDir::new().unwrap();
    let outside_file = outside.path().join("secret.txt");
    fs::write(&outside_file, "outside").unwrap();
    let safe = workspace.path().join("safe.txt");
    let target = workspace.path().join("target.txt");
    fs::write(&safe, "inside").unwrap();
    symlink(&outside_file, &target).unwrap();

    let safe = CString::new(safe.as_os_str().as_bytes()).unwrap();
    let target = CString::new(target.as_os_str().as_bytes()).unwrap();
    let running = Arc::new(AtomicBool::new(true));
    let swap_running = Arc::clone(&running);
    let swapper = thread::spawn(move || {
        while swap_running.load(Ordering::Relaxed) {
            let result = unsafe {
                libc::renameat2(
                    libc::AT_FDCWD,
                    safe.as_ptr(),
                    libc::AT_FDCWD,
                    target.as_ptr(),
                    libc::RENAME_EXCHANGE,
                )
            };
            assert_eq!(result, 0);
        }
    });

    let tool = ReadFileTool::new(workspace.path());
    for _ in 0..2_000 {
        if let Ok(content) = execute(&tool, "target.txt") {
            assert_eq!(content, "inside");
        }
    }
    running.store(false, Ordering::Relaxed);
    swapper.join().unwrap();
}

fn json_read(tool: &ReadFileTool, args: serde_json::Value) -> serde_json::Value {
    let result = tool.execute(&args).unwrap();
    assert!(result.message.len() <= MAX_OUTPUT_BYTES);
    serde_json::from_str(&result.message).unwrap()
}

#[test]
fn reads_an_inclusive_line_range_with_original_newlines_and_numbers() {
    let workspace = TempDir::new().unwrap();
    fs::write(
        workspace.path().join("notes.txt"),
        "first\r\nViệt 🌍\r\nthird\nlast",
    )
    .unwrap();
    let tool = ReadFileTool::new(workspace.path());
    let page = json_read(
        &tool,
        serde_json::json!({
            "path":"notes.txt", "start_line":2, "end_line":3,
            "line_numbers":true, "format":"json"
        }),
    );
    assert_eq!(page["data"]["content"], "2: Việt 🌍\r\n3: third\n");
    assert_eq!(page["data"]["start_byte"], 7);
    assert_eq!(page["data"]["start_line"], 2);
    assert_eq!(page["truncated"], false);
    assert!(page["next_cursor"].is_null());
}

#[test]
fn reads_lines_after_the_old_64k_prefix() {
    let workspace = TempDir::new().unwrap();
    let mut content = "prefix\n".repeat(12_000);
    content.push_str("wanted\nend");
    fs::write(workspace.path().join("large.txt"), content).unwrap();
    let page = json_read(
        &ReadFileTool::new(workspace.path()),
        serde_json::json!({
            "path":"large.txt", "start_line":12001, "end_line":12001, "format":"json"
        }),
    );
    assert_eq!(page["data"]["content"], "wanted\n");
    assert_eq!(page["truncated"], false);
}

#[test]
fn empty_ranges_at_eof_do_not_invent_a_line() {
    let workspace = TempDir::new().unwrap();
    fs::write(workspace.path().join("notes.txt"), "one\ntwo\n").unwrap();
    let tool = ReadFileTool::new(workspace.path());
    for start in [3, 9] {
        let page = json_read(
            &tool,
            serde_json::json!({
                "path":"notes.txt", "start_line":start, "format":"json", "line_numbers":true
            }),
        );
        assert_eq!(page["data"]["content"], "");
        assert_eq!(page["truncated"], false);
        assert!(page["next_cursor"].is_null());
    }
}

#[test]
fn json_byte_cursor_resumes_long_unicode_lines_without_loss() {
    let workspace = TempDir::new().unwrap();
    let content = format!("{}\nlast", "\"🌍\t".repeat(20_000));
    fs::write(workspace.path().join("long.txt"), &content).unwrap();
    let tool = ReadFileTool::new(workspace.path());
    let mut offset = 0;
    let mut recovered = String::new();
    loop {
        let page = json_read(
            &tool,
            serde_json::json!({
                "path":"long.txt", "byte_offset":offset, "format":"json"
            }),
        );
        recovered.push_str(page["data"]["content"].as_str().unwrap());
        if !page["truncated"].as_bool().unwrap() {
            assert!(page["next_cursor"].is_null());
            break;
        }
        let next = page["next_cursor"]["byte_offset"].as_u64().unwrap();
        assert!(next > offset);
        offset = next;
    }
    assert_eq!(recovered, content);
}

#[test]
fn numbered_output_is_bounded_even_for_many_empty_lines() {
    let workspace = TempDir::new().unwrap();
    fs::write(workspace.path().join("blank.txt"), "\n".repeat(30_000)).unwrap();
    let page = json_read(
        &ReadFileTool::new(workspace.path()),
        serde_json::json!({
            "path":"blank.txt", "line_numbers":true, "format":"json"
        }),
    );
    assert_eq!(page["truncated"], true);
    assert!(page["data"]["content"]
        .as_str()
        .unwrap()
        .starts_with("1: \n2: \n"));
    assert!(page["next_cursor"]["byte_offset"].as_u64().unwrap() > 0);
}

#[test]
fn rejects_invalid_ranges_offsets_formats_and_unknown_arguments() {
    let workspace = TempDir::new().unwrap();
    fs::write(workspace.path().join("note.txt"), "🌍").unwrap();
    let tool = ReadFileTool::new(workspace.path());
    for extra in [
        serde_json::json!({"start_line":0}),
        serde_json::json!({"start_line":3,"end_line":2}),
        serde_json::json!({"end_line":0}),
        serde_json::json!({"byte_offset":1}),
        serde_json::json!({"byte_offset":0,"start_line":1}),
        serde_json::json!({"byte_offset":0,"line_numbers":true}),
        serde_json::json!({"format":"yaml"}),
        serde_json::json!({"start_lien":2}),
    ] {
        let mut args = extra;
        args["path"] = "note.txt".into();
        assert!(tool.execute(&args).is_err(), "accepted {args}");
    }
}

#[test]
fn range_scanning_is_bounded_with_actionable_error() {
    let workspace = TempDir::new().unwrap();
    fs::write(
        workspace.path().join("huge.txt"),
        vec![b'a'; 32 * 1024 * 1024 + 1],
    )
    .unwrap();
    let error = ReadFileTool::new(workspace.path())
        .execute(&serde_json::json!({
            "path":"huge.txt", "start_line":2
        }))
        .err()
        .unwrap();
    assert!(error.contains("byte_offset"), "{error}");
}

#[test]
fn byte_cursor_preserves_the_end_of_a_truncated_line_range() {
    let workspace = TempDir::new().unwrap();
    let expected = format!("{}\nsecond\n", "🌍".repeat(20_000));
    fs::write(
        workspace.path().join("range.txt"),
        format!("{expected}excluded\n"),
    )
    .unwrap();
    let tool = ReadFileTool::new(workspace.path());
    let mut args = serde_json::json!({"path":"range.txt","end_line":2,"format":"json"});
    let mut recovered = String::new();
    loop {
        let result = json_read(&tool, args);
        recovered.push_str(result["data"]["content"].as_str().unwrap());
        if !result["truncated"].as_bool().unwrap() {
            break;
        }
        args = result["next_cursor"].clone();
        args["path"] = "range.txt".into();
    }
    assert_eq!(recovered, expected);
}
