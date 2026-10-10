use std::fs;

use atlas_agentic::tools::{SearchFilesTool, Tool};
use serde_json::json;

#[test]
fn searches_file_names_and_utf8_contents_in_deterministic_order() {
    let workspace = tempfile::tempdir().unwrap();
    fs::create_dir(workspace.path().join("docs")).unwrap();
    fs::write(workspace.path().join("alpha.txt"), "first needle\nsecond").unwrap();
    fs::write(workspace.path().join("docs/needle.md"), "no content match").unwrap();
    fs::write(workspace.path().join("docs/zeta.txt"), "needle here").unwrap();

    let output = SearchFilesTool::new(workspace.path())
        .execute(&json!({"query":"needle"}))
        .unwrap()
        .message;

    let lines = output.lines().collect::<Vec<_>>();
    assert_eq!(lines[0], "content alpha.txt:1: first needle");
    assert_eq!(lines[1], "filename docs/needle.md");
    assert_eq!(lines[2], "content docs/zeta.txt:1: needle here");
}

#[test]
fn search_respects_root_glob_hidden_generated_binary_and_limit() {
    let workspace = tempfile::tempdir().unwrap();
    fs::create_dir_all(workspace.path().join("docs/nested")).unwrap();
    fs::create_dir(workspace.path().join("target")).unwrap();
    fs::write(workspace.path().join("docs/a.md"), "match one").unwrap();
    fs::write(workspace.path().join("docs/nested/b.txt"), "match two").unwrap();
    fs::write(workspace.path().join("docs/.hidden.md"), "match hidden").unwrap();
    fs::write(
        workspace.path().join("target/generated.md"),
        "match generated",
    )
    .unwrap();
    fs::write(workspace.path().join("docs/binary.md"), b"match\0binary").unwrap();

    let tool = SearchFilesTool::new(workspace.path());
    let output = tool
        .execute(&json!({
            "query":"match",
            "root":"docs",
            "glob":"*.md",
            "limit":1
        }))
        .unwrap()
        .message;
    assert_eq!(output.lines().count(), 1);
    assert!(output.contains("docs/a.md"), "{output}");
    assert!(!output.contains("hidden"));
    assert!(!output.contains("binary"));

    let hidden = tool
        .execute(&json!({
            "query":"match hidden",
            "root":"docs",
            "includeHidden":true
        }))
        .unwrap()
        .message;
    assert!(hidden.contains("docs/.hidden.md"), "{hidden}");
}

#[test]
fn search_validates_arguments_and_reports_no_matches() {
    let workspace = tempfile::tempdir().unwrap();
    let tool = SearchFilesTool::new(workspace.path());

    assert!(tool
        .execute(&json!({"query":""}))
        .err()
        .unwrap()
        .contains("query is required"));
    assert!(tool
        .execute(&json!({"query":"x","root":"../outside"}))
        .is_err());
    assert!(tool
        .execute(&json!({"query":"x","limit":0}))
        .err()
        .unwrap()
        .contains("limit"));
    assert_eq!(
        tool.execute(&json!({"query":"missing"})).unwrap().message,
        "No matches."
    );
}

#[cfg(unix)]
#[test]
fn search_does_not_follow_symlink_directories() {
    use std::os::unix::fs::symlink;

    let root = tempfile::tempdir().unwrap();
    let workspace = root.path().join("workspace");
    let outside = root.path().join("outside");
    fs::create_dir(&workspace).unwrap();
    fs::create_dir(&outside).unwrap();
    fs::write(outside.join("secret.txt"), "needle").unwrap();
    symlink(&outside, workspace.join("escape")).unwrap();

    assert_eq!(
        SearchFilesTool::new(&workspace)
            .execute(&json!({"query":"needle"}))
            .unwrap()
            .message,
        "No matches."
    );
}
