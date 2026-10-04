use std::fs;

use atlas_agentic::tools::{ReadFileTool, Tool};
use tempfile::TempDir;

const MAX_OUTPUT_BYTES: usize = 64 * 1024;

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
        ("missing.txt", "failed to resolve read_file target"),
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
    let mut content = "a".repeat(MAX_OUTPUT_BYTES - 2);
    content.push('🌍');
    content.push_str(&"b".repeat(64));
    fs::write(workspace.path().join("large.txt"), content).unwrap();

    let output = execute(&ReadFileTool::new(workspace.path()), "large.txt").unwrap();

    assert!(output.len() <= MAX_OUTPUT_BYTES);
    assert!(output.ends_with("\n[read_file output truncated]"));
    assert!(std::str::from_utf8(output.as_bytes()).is_ok());
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

    assert_eq!(
        execute(&ReadFileTool::new(workspace.path()), "secret-link.txt").unwrap_err(),
        "read_file target escapes the workspace"
    );
}
