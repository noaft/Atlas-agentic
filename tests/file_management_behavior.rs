use std::fs;

use atlas_agentic::{
    permissions::ToolRisk,
    tools::{
        CopyFileTool, CreateDirectoryTool, CreateFileTool, DeleteFileTool, ListDirTool,
        MoveFileTool, Tool,
    },
};
use serde_json::json;

#[test]
fn create_copy_move_and_delete_files_inside_the_workspace() {
    let workspace = tempfile::tempdir().unwrap();
    let create_directory = CreateDirectoryTool::new(workspace.path());
    let copy = CopyFileTool::new(workspace.path());
    let move_file = MoveFileTool::new(workspace.path());
    let delete = DeleteFileTool::new(workspace.path());

    create_directory
        .execute(&json!({"path":"notes/archive"}))
        .unwrap();
    fs::write(workspace.path().join("notes/source.txt"), "hello").unwrap();
    copy.execute(&json!({
        "source":"notes/source.txt",
        "destination":"notes/copied.txt"
    }))
    .unwrap();
    move_file
        .execute(&json!({
            "source":"notes/copied.txt",
            "destination":"notes/archive/moved.txt"
        }))
        .unwrap();
    delete
        .execute(&json!({"path":"notes/archive/moved.txt"}))
        .unwrap();

    assert_eq!(
        fs::read_to_string(workspace.path().join("notes/source.txt")).unwrap(),
        "hello"
    );
    assert!(!workspace.path().join("notes/copied.txt").exists());
    assert!(!workspace.path().join("notes/archive/moved.txt").exists());
}

#[test]
fn copy_and_move_refuse_overwrite_unless_explicit() {
    let workspace = tempfile::tempdir().unwrap();
    fs::write(workspace.path().join("source.txt"), "new").unwrap();
    fs::write(workspace.path().join("target.txt"), "old").unwrap();
    let copy = CopyFileTool::new(workspace.path());

    let error = copy
        .execute(&json!({"source":"source.txt","destination":"target.txt"}))
        .err()
        .unwrap();
    assert!(error.contains("already exists"), "{error}");
    assert_eq!(
        fs::read_to_string(workspace.path().join("target.txt")).unwrap(),
        "old"
    );
    assert_eq!(
        copy.risk(&json!({
            "source":"source.txt",
            "destination":"target.txt",
            "overwrite":true
        }))
        .unwrap(),
        ToolRisk::Destructive
    );
    copy.execute(&json!({
        "source":"source.txt",
        "destination":"target.txt",
        "overwrite":true
    }))
    .unwrap();
    assert_eq!(
        fs::read_to_string(workspace.path().join("target.txt")).unwrap(),
        "new"
    );

    let move_file = MoveFileTool::new(workspace.path());
    fs::write(workspace.path().join("second.txt"), "second").unwrap();
    assert!(move_file
        .execute(&json!({"source":"second.txt","destination":"target.txt"}))
        .err()
        .unwrap()
        .contains("already exists"));
    assert!(workspace.path().join("second.txt").exists());
}

#[test]
fn copy_and_move_reject_hard_link_aliases_even_with_overwrite() {
    let workspace = tempfile::tempdir().unwrap();
    let source = workspace.path().join("source.txt");
    let alias = workspace.path().join("alias.txt");
    fs::write(&source, "keep me").unwrap();
    fs::hard_link(&source, &alias).unwrap();

    let copy = CopyFileTool::new(workspace.path());
    let copy_error = copy
        .execute(&json!({
            "source":"source.txt",
            "destination":"alias.txt",
            "overwrite":true
        }))
        .err()
        .unwrap();
    assert!(
        copy_error.contains("must be different files"),
        "{copy_error}"
    );

    let move_file = MoveFileTool::new(workspace.path());
    let move_error = move_file
        .execute(&json!({
            "source":"source.txt",
            "destination":"alias.txt",
            "overwrite":true
        }))
        .err()
        .unwrap();
    assert!(
        move_error.contains("must be different files"),
        "{move_error}"
    );
    assert_eq!(fs::read_to_string(&source).unwrap(), "keep me");
    assert_eq!(fs::read_to_string(&alias).unwrap(), "keep me");
}

#[test]
fn file_tools_reject_unsafe_paths_directories_and_same_target() {
    let workspace = tempfile::tempdir().unwrap();
    fs::create_dir(workspace.path().join("directory")).unwrap();
    fs::write(workspace.path().join("file.txt"), "text").unwrap();
    let copy = CopyFileTool::new(workspace.path());
    let move_file = MoveFileTool::new(workspace.path());
    let delete = DeleteFileTool::new(workspace.path());

    for path in ["../outside.txt", "/tmp/outside.txt"] {
        assert!(delete.execute(&json!({"path":path})).is_err());
    }
    assert!(delete
        .execute(&json!({"path":"directory"}))
        .err()
        .unwrap()
        .contains("not a file"));
    assert!(copy
        .execute(&json!({"source":"directory","destination":"copy"}))
        .err()
        .unwrap()
        .contains("not a file"));
    assert!(move_file
        .execute(&json!({"source":"file.txt","destination":"file.txt"}))
        .err()
        .unwrap()
        .contains("must be different"));
}

#[cfg(unix)]
#[test]
fn file_tools_do_not_follow_symlinks_outside_the_workspace() {
    use std::os::unix::fs::symlink;

    let root = tempfile::tempdir().unwrap();
    let workspace = root.path().join("workspace");
    let outside = root.path().join("outside");
    fs::create_dir(&workspace).unwrap();
    fs::create_dir(&outside).unwrap();
    fs::write(outside.join("secret.txt"), "secret").unwrap();
    symlink(&outside, workspace.join("escape")).unwrap();

    let copy = CopyFileTool::new(&workspace);
    let create = CreateFileTool::new(&workspace);
    let delete = DeleteFileTool::new(&workspace);
    let list = ListDirTool::new(&workspace);
    assert!(copy
        .execute(&json!({"source":"escape/secret.txt","destination":"copy.txt"}))
        .is_err());
    assert!(delete
        .execute(&json!({"path":"escape/secret.txt"}))
        .is_err());
    assert!(create
        .execute(&json!({"path":"escape/created.txt","content":"bad"}))
        .is_err());
    assert!(list.execute(&json!({"path":"escape"})).is_err());
    assert!(outside.join("secret.txt").exists());
    assert!(!outside.join("created.txt").exists());
}

#[test]
fn file_tool_definitions_and_risks_are_stable() {
    let workspace = tempfile::tempdir().unwrap();
    let directory = CreateDirectoryTool::new(workspace.path());
    let copy = CopyFileTool::new(workspace.path());
    let move_file = MoveFileTool::new(workspace.path());
    let delete = DeleteFileTool::new(workspace.path());

    assert_eq!(directory.definition().name, "create_directory");
    assert_eq!(copy.definition().name, "copy_file");
    assert_eq!(move_file.definition().name, "move_file");
    assert_eq!(delete.definition().name, "delete_file");
    assert_eq!(
        directory.risk(&json!({"path":"notes"})).unwrap(),
        ToolRisk::ModifyWorkspace
    );
    assert_eq!(
        delete.risk(&json!({"path":"note.txt"})).unwrap(),
        ToolRisk::Destructive
    );
}
