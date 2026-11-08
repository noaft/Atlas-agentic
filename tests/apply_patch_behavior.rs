use atlas_agentic::{
    agent_output::AgentOutput,
    permissions::ToolRisk,
    tools::{ApplyPatchTool, Tool, ToolRegistry},
};
use serde_json::json;
use std::fs;
fn args(old: &str, new: &str) -> serde_json::Value {
    json!({"path":"note.txt","edits":[{"old_text":old,"new_text":new}]})
}
#[test]
fn precise_utf8_edit_preserves_other_bytes_and_reports_hashes() {
    let root = tempfile::tempdir().unwrap();
    fs::write(root.path().join("note.txt"), "Xin chào\r\nkeep\r\n").unwrap();
    let tool = ApplyPatchTool::new(root.path());
    assert_eq!(
        tool.risk(&args("Xin chào", "Tiếng Việt")).unwrap(),
        ToolRisk::Destructive
    );
    let result = tool.execute(&args("Xin chào", "Tiếng Việt")).unwrap();
    let result: serde_json::Value = serde_json::from_str(&result.message).unwrap();
    assert_eq!(
        fs::read_to_string(root.path().join("note.txt")).unwrap(),
        "Tiếng Việt\r\nkeep\r\n"
    );
    assert_eq!(result["replacements"], 1);
    assert_ne!(result["before_sha256"], result["after_sha256"]);
}
#[test]
fn invalid_or_ambiguous_edits_never_publish_partial_changes() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("note.txt");
    fs::write(&path, "one one\ntwo").unwrap();
    let tool = ApplyPatchTool::new(root.path());
    for input in [
        args("one", "x"),
        args("missing", "x"),
        args("", "x"),
        json!({"path":"note.txt","edits":[{"old_text":"two","new_text":"changed"},{"old_text":"missing","new_text":"bad"}]}),
    ] {
        assert!(tool.execute(&input).is_err());
        assert_eq!(fs::read_to_string(&path).unwrap(), "one one\ntwo");
    }
    for path in ["../outside", "/tmp/outside", ""] {
        assert!(tool
            .execute(&json!({"path":path,"edits":[{"old_text":"two","new_text":"x"}]}))
            .is_err());
    }
    assert_eq!(fs::read_dir(root.path()).unwrap().count(), 1);
}
#[test]
fn approval_binds_the_entire_preimage() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("note.txt");
    fs::write(&path, "original\nkeep").unwrap();
    let mut registry = ToolRegistry::empty();
    registry.register(Box::new(ApplyPatchTool::new(root.path())));
    let input = args("original", "updated");
    let approval = registry.authorization("apply_patch", &input).unwrap();
    fs::write(&path, "original\nexternal edit").unwrap();
    assert!(registry
        .execute_authorized(
            &AgentOutput::ToolCall {
                tool: "apply_patch".into(),
                args: input,
                message: None
            },
            &approval
        )
        .is_err());
    assert_eq!(
        fs::read_to_string(&path).unwrap(),
        "original\nexternal edit"
    );
}
#[cfg(unix)]
#[test]
fn symlinks_special_files_and_outside_parents_are_rejected_and_mode_is_preserved() {
    use std::os::unix::fs::{symlink, PermissionsExt};
    let root = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    fs::write(outside.path().join("note.txt"), "original").unwrap();
    symlink(
        outside.path().join("note.txt"),
        root.path().join("note.txt"),
    )
    .unwrap();
    let tool = ApplyPatchTool::new(root.path());
    assert!(tool.execute(&args("original", "bad")).is_err());
    fs::remove_file(root.path().join("note.txt")).unwrap();
    symlink(outside.path(), root.path().join("linked")).unwrap();
    assert!(tool
        .execute(
            &json!({"path":"linked/note.txt","edits":[{"old_text":"original","new_text":"bad"}]})
        )
        .is_err());
    fs::write(root.path().join("note.txt"), "original").unwrap();
    fs::set_permissions(
        root.path().join("note.txt"),
        fs::Permissions::from_mode(0o640),
    )
    .unwrap();
    tool.execute(&args("original", "updated")).unwrap();
    assert_eq!(
        fs::metadata(root.path().join("note.txt"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o640
    );
    assert_eq!(
        fs::read_to_string(outside.path().join("note.txt")).unwrap(),
        "original"
    );
}

#[test]
fn overlapping_matches_are_ambiguous_and_old_inode_snapshots_cannot_overwrite() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("note.txt");
    fs::write(&path, "aaa").unwrap();
    let tool = ApplyPatchTool::new(root.path());
    assert!(tool.execute(&args("aa", "x")).is_err());
    fs::write(&path, "original").unwrap();
    let mut registry = ToolRegistry::empty();
    registry.register(Box::new(tool));
    let first = args("original", "first");
    let second = args("original", "second");
    let a = registry.authorization("apply_patch", &first).unwrap();
    let b = registry.authorization("apply_patch", &second).unwrap();
    registry
        .execute_authorized(
            &AgentOutput::ToolCall {
                tool: "apply_patch".into(),
                args: first,
                message: None,
            },
            &a,
        )
        .unwrap();
    assert!(registry
        .execute_authorized(
            &AgentOutput::ToolCall {
                tool: "apply_patch".into(),
                args: second,
                message: None
            },
            &b
        )
        .is_err());
    assert_eq!(fs::read_to_string(&path).unwrap(), "first");
}
#[test]
fn oversized_binary_and_empty_edit_inputs_are_rejected() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("note.txt");
    let tool = ApplyPatchTool::new(root.path());
    fs::write(&path, [255, 254]).unwrap();
    assert!(tool.execute(&args("x", "y")).is_err());
    fs::write(&path, "x".repeat(1024 * 1024 + 1)).unwrap();
    assert!(tool.execute(&args("x", "y")).is_err());
    fs::write(&path, "small").unwrap();
    assert!(tool
        .execute(&json!({"path":"note.txt","edits":[]}))
        .is_err());
    assert!(tool
        .execute(&args("small", &"x".repeat(1024 * 1024)))
        .is_err());
    assert_eq!(fs::read_to_string(&path).unwrap(), "small");
}
