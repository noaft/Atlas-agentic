use std::{fs, process::Command, time::Duration};

use atlas_agentic::{
    agent_output::AgentOutput,
    permissions::ToolRisk,
    tools::{RipgrepTool, Tool, ToolRegistry},
};
use serde_json::json;

#[test]
fn definition_exposes_the_bounded_native_search_contract() {
    let workspace = tempfile::tempdir().unwrap();
    let definition = RipgrepTool::new(workspace.path()).definition();

    assert_eq!(definition.name, "ripgrep");
    assert!(definition.description.contains("native"));
    assert_eq!(definition.input_schema["required"], json!(["pattern"]));
    assert_eq!(definition.input_schema["additionalProperties"], false);
    assert_eq!(
        definition.input_schema["properties"]["path"]["default"],
        "."
    );
    assert_eq!(
        definition.input_schema["properties"]["fixedStrings"]["default"],
        false
    );
    assert_eq!(
        definition.input_schema["properties"]["caseSensitive"]["default"],
        true
    );
    assert_eq!(
        definition.input_schema["properties"]["includeHidden"]["default"],
        false
    );
    assert_eq!(
        definition.input_schema["properties"]["includeIgnored"]["default"],
        false
    );
    assert_eq!(
        definition.input_schema["properties"]["limit"]["default"],
        50
    );
    assert_eq!(definition.input_schema["properties"]["limit"]["minimum"], 1);
    assert_eq!(
        definition.input_schema["properties"]["limit"]["maximum"],
        200
    );
}

#[test]
fn searches_regex_recursively_in_deterministic_order() {
    let workspace = tempfile::tempdir().unwrap();
    fs::create_dir(workspace.path().join("docs")).unwrap();
    fs::write(workspace.path().join("alpha.txt"), "first needle\nsecond\n").unwrap();
    fs::write(workspace.path().join("docs/beta.txt"), "needful here\n").unwrap();
    fs::write(workspace.path().join("docs/zeta.txt"), "needle again\n").unwrap();

    let output = RipgrepTool::new(workspace.path())
        .execute(&json!({"pattern":"need(?:le|ful)"}))
        .unwrap()
        .message;

    assert_eq!(
        output,
        "alpha.txt:1:first needle\ndocs/beta.txt:1:needful here\ndocs/zeta.txt:1:needle again"
    );
}

#[test]
fn searches_a_single_file_root() {
    let workspace = tempfile::tempdir().unwrap();
    fs::write(workspace.path().join("one.txt"), "before\nneedle\n").unwrap();

    let output = RipgrepTool::new(workspace.path())
        .execute(&json!({"pattern":"needle", "path":"one.txt"}))
        .unwrap()
        .message;

    assert_eq!(output, "one.txt:2:needle");
}

#[test]
fn supports_fixed_strings_case_and_glob_controls() {
    let workspace = tempfile::tempdir().unwrap();
    fs::write(workspace.path().join("notes.md"), "a+b\nA+B\n").unwrap();
    fs::write(workspace.path().join("notes.txt"), "a+b\n").unwrap();
    let tool = RipgrepTool::new(workspace.path());

    let exact = tool
        .execute(&json!({
            "pattern":"a+b",
            "fixedStrings":true,
            "glob":"*.md"
        }))
        .unwrap()
        .message;
    assert_eq!(exact, "notes.md:1:a+b");

    let insensitive = tool
        .execute(&json!({
            "pattern":"a+b",
            "fixedStrings":true,
            "caseSensitive":false,
            "glob":"*.md"
        }))
        .unwrap()
        .message;
    assert_eq!(insensitive, "notes.md:1:a+b\nnotes.md:2:A+B");

    let excluded = tool
        .execute(&json!({
            "pattern":"a+b",
            "fixedStrings":true,
            "glob":"!*.txt"
        }))
        .unwrap()
        .message;
    assert_eq!(excluded, "notes.md:1:a+b");

    assert_eq!(
        tool.execute(&json!({"pattern":"missing"})).unwrap().message,
        "No matches."
    );
}

#[test]
fn rejects_unknown_fields_invalid_patterns_globs_limits_and_paths() {
    let workspace = tempfile::tempdir().unwrap();
    let tool = RipgrepTool::new(workspace.path());

    for (args, expected) in [
        (json!({"pattern":"x", "surprise":true}), "unknown field"),
        (json!({"pattern":""}), "pattern is required"),
        (json!({"pattern":"["}), "invalid ripgrep pattern"),
        (json!({"pattern":"x", "glob":"["}), "invalid ripgrep glob"),
        (json!({"pattern":"x", "limit":0}), "limit must be between"),
        (
            json!({"pattern":"x", "path":"missing"}),
            "failed to resolve ripgrep path",
        ),
    ] {
        let error = tool.execute(&args).err().unwrap();
        assert!(
            error.contains(expected),
            "{error:?} did not contain {expected:?}"
        );
    }
}

#[test]
fn respects_hidden_ignore_binary_and_symlink_defaults() {
    let root = tempfile::tempdir().unwrap();
    let workspace = root.path().join("workspace");
    let outside = root.path().join("outside");
    fs::create_dir(&workspace).unwrap();
    fs::create_dir(&outside).unwrap();
    fs::write(workspace.join("visible.txt"), "needle visible\n").unwrap();
    fs::write(workspace.join(".hidden.txt"), "needle hidden\n").unwrap();
    fs::write(workspace.join(".ignore"), "ignored.txt\n").unwrap();
    fs::write(workspace.join(".rgignore"), "rgignored.txt\n").unwrap();
    fs::write(workspace.join("ignored.txt"), "needle ignored\n").unwrap();
    fs::write(workspace.join("rgignored.txt"), "needle rgignored\n").unwrap();
    fs::write(workspace.join("binary.dat"), b"\0needle binary\n").unwrap();
    fs::write(outside.join("secret.txt"), "needle outside\n").unwrap();

    #[cfg(unix)]
    std::os::unix::fs::symlink(&outside, workspace.join("outside-link")).unwrap();

    let output = RipgrepTool::new(&workspace)
        .execute(&json!({"pattern":"needle"}))
        .unwrap()
        .message;

    assert_eq!(output, "visible.txt:1:needle visible");
}

#[test]
fn include_flags_can_search_hidden_and_ignored_text() {
    let workspace = tempfile::tempdir().unwrap();
    fs::write(workspace.path().join(".hidden.txt"), "needle hidden\n").unwrap();
    fs::write(workspace.path().join(".ignore"), "ignored.txt\n").unwrap();
    fs::write(workspace.path().join(".rgignore"), "rgignored.txt\n").unwrap();
    fs::write(workspace.path().join("ignored.txt"), "needle ignored\n").unwrap();
    fs::write(workspace.path().join("rgignored.txt"), "needle rgignored\n").unwrap();

    let output = RipgrepTool::new(workspace.path())
        .execute(&json!({
            "pattern":"needle",
            "includeHidden":true,
            "includeIgnored":true
        }))
        .unwrap()
        .message;

    assert_eq!(
        output,
        ".hidden.txt:1:needle hidden\nignored.txt:1:needle ignored\nrgignored.txt:1:needle rgignored"
    );
}

#[test]
fn bounds_match_count_and_utf8_output_bytes() {
    let workspace = tempfile::tempdir().unwrap();
    let long_line = format!("needle {}\n", "é".repeat(400));
    fs::write(workspace.path().join("large.txt"), long_line.repeat(200)).unwrap();

    let output = RipgrepTool::new(workspace.path())
        .execute(&json!({"pattern":"needle", "limit":200}))
        .unwrap()
        .message;

    assert!(output.len() <= 65_536, "result was {} bytes", output.len());
    assert!(!output.contains('\u{FFFD}'));
    assert!(output.ends_with("[results truncated]"), "{output}");

    let limited = RipgrepTool::new(workspace.path())
        .execute(&json!({"pattern":"needle", "limit":2}))
        .unwrap()
        .message;
    assert_eq!(limited.lines().count(), 3);
    assert!(limited.ends_with("[results truncated]"));
}

#[test]
fn classifies_canonical_inside_and_outside_roots() {
    let root = tempfile::tempdir().unwrap();
    let workspace = root.path().join("workspace");
    let outside = root.path().join("outside");
    fs::create_dir(&workspace).unwrap();
    fs::create_dir(&outside).unwrap();
    fs::write(workspace.join("inside.txt"), "x").unwrap();
    let tool = RipgrepTool::new(&workspace);

    assert_eq!(
        tool.risk(&json!({"pattern":"x"})).unwrap(),
        ToolRisk::ReadOnly
    );
    assert_eq!(
        tool.risk(&json!({"pattern":"x", "path":workspace.join("inside.txt")}))
            .unwrap(),
        ToolRisk::ReadOnly
    );
    assert_eq!(
        tool.risk(&json!({"pattern":"x", "path":outside})).unwrap(),
        ToolRisk::ReadOutsideWorkspace
    );
}

#[cfg(unix)]
#[test]
fn relative_symlink_root_is_classified_outside() {
    use std::os::unix::fs::symlink;

    let root = tempfile::tempdir().unwrap();
    let workspace = root.path().join("workspace");
    let outside = root.path().join("outside");
    fs::create_dir(&workspace).unwrap();
    fs::create_dir(&outside).unwrap();
    symlink(&outside, workspace.join("linked")).unwrap();

    assert_eq!(
        RipgrepTool::new(&workspace)
            .risk(&json!({"pattern":"x", "path":"linked"}))
            .unwrap(),
        ToolRisk::ReadOutsideWorkspace
    );
}

#[cfg(unix)]
#[test]
fn rejects_a_root_retargeted_after_authorization() {
    use std::os::unix::fs::symlink;

    let root = tempfile::tempdir().unwrap();
    let workspace = root.path().join("workspace");
    let inside = workspace.join("inside");
    let outside = root.path().join("outside");
    fs::create_dir_all(&inside).unwrap();
    fs::create_dir(&outside).unwrap();
    fs::write(inside.join("note.txt"), "inside needle\n").unwrap();
    fs::write(outside.join("secret.txt"), "outside needle\n").unwrap();
    let link = workspace.join("linked");
    symlink(&inside, &link).unwrap();

    let registry = ToolRegistry::new(vec![Box::new(RipgrepTool::new(&workspace))]);
    let args = json!({"pattern":"needle", "path":"linked"});
    let authorization = registry.authorization("ripgrep", &args).unwrap();
    assert_eq!(authorization.risk(), ToolRisk::ReadOnly);

    fs::remove_file(&link).unwrap();
    symlink(&outside, &link).unwrap();

    let error = registry
        .execute_authorized(
            &AgentOutput::ToolCall {
                tool: "ripgrep".to_string(),
                args,
            },
            &authorization,
        )
        .err()
        .expect("retargeted root was searched");
    assert!(error.contains("changed after authorization"), "{error}");
}

#[cfg(unix)]
#[test]
fn rejects_a_special_file_root_without_blocking() {
    use std::{ffi::CString, os::unix::ffi::OsStrExt, sync::mpsc, thread};

    let workspace = tempfile::tempdir().unwrap();
    let fifo = workspace.path().join("pipe");
    let fifo_c = CString::new(fifo.as_os_str().as_bytes()).unwrap();
    assert_eq!(unsafe { libc::mkfifo(fifo_c.as_ptr(), 0o600) }, 0);
    let workspace_path = workspace.path().to_path_buf();
    let (sender, receiver) = mpsc::channel();

    thread::spawn(move || {
        let result = RipgrepTool::new(workspace_path).execute(&json!({
            "pattern":"needle",
            "path":"pipe"
        }));
        sender.send(result.err()).unwrap();
    });

    let error = receiver
        .recv_timeout(Duration::from_secs(2))
        .expect("ripgrep blocked while opening a FIFO")
        .expect("ripgrep accepted a FIFO search root");
    assert!(error.contains("regular file or directory"), "{error}");
}

#[test]
fn works_with_an_empty_path_environment() {
    const CHILD_MARKER: &str = "ATLAS_RIPGREP_EMPTY_PATH_HELPER";
    if std::env::var_os(CHILD_MARKER).is_some() {
        let workspace = tempfile::tempdir().unwrap();
        fs::write(workspace.path().join("note.txt"), "needle\n").unwrap();
        let output = RipgrepTool::new(workspace.path())
            .execute(&json!({"pattern":"needle"}))
            .unwrap()
            .message;
        assert_eq!(output, "note.txt:1:needle");
        return;
    }

    let status = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "works_with_an_empty_path_environment",
            "--nocapture",
        ])
        .env(CHILD_MARKER, "1")
        .env("PATH", "")
        .status()
        .unwrap();
    assert!(status.success());
}
