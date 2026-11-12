use std::fs;

use atlas_agentic::tools::{ListDirTool, Tool};
use serde_json::{json, Value};
use tempfile::TempDir;

fn page(tool: &ListDirTool, args: Value) -> Value {
    let result = tool.execute(&args).unwrap();
    assert!(result.message.len() <= 64 * 1024);
    serde_json::from_str(&result.message).unwrap()
}

#[test]
fn small_default_text_lists_remain_sorted_and_compatible() {
    let workspace = TempDir::new().unwrap();
    fs::create_dir(workspace.path().join("notes")).unwrap();
    fs::write(workspace.path().join("notes/b.txt"), "b").unwrap();
    fs::write(workspace.path().join("notes/a.txt"), "a").unwrap();
    let result = ListDirTool::new(workspace.path())
        .execute(&json!({"path":"notes"}))
        .unwrap();
    assert_eq!(result.message, "notes/a.txt\nnotes/b.txt");
}

#[test]
fn json_pages_classify_entries_and_resume_in_sorted_order() {
    let workspace = TempDir::new().unwrap();
    fs::create_dir(workspace.path().join("b-dir")).unwrap();
    fs::write(workspace.path().join("a-file"), "a").unwrap();
    fs::write(workspace.path().join("c-file"), "c").unwrap();
    let tool = ListDirTool::new(workspace.path());
    let first = page(&tool, json!({"limit":2,"format":"json"}));
    assert_eq!(
        first["data"]["entries"],
        json!([
            {"path":"a-file","kind":"file"}, {"path":"b-dir","kind":"directory"}
        ])
    );
    assert_eq!(first["truncated"], true);
    assert_eq!(first["next_cursor"]["offset"], 2);
    let last = page(&tool, first["next_cursor"].clone());
    assert_eq!(
        last["data"]["entries"],
        json!([{"path":"c-file","kind":"file"}])
    );
    assert_eq!(last["truncated"], false);
    assert!(last["next_cursor"].is_null());
}

#[test]
fn default_page_is_bounded_for_large_directories() {
    let workspace = TempDir::new().unwrap();
    for index in (0..105).rev() {
        fs::write(workspace.path().join(format!("file-{index:03}")), "").unwrap();
    }
    let tool = ListDirTool::new(workspace.path());
    let first = page(&tool, json!({"format":"json"}));
    assert_eq!(first["data"]["entries"].as_array().unwrap().len(), 100);
    assert_eq!(first["data"]["entries"][0]["path"], "file-000");
    assert_eq!(first["data"]["entries"][99]["path"], "file-099");
    let last = page(&tool, first["next_cursor"].clone());
    assert_eq!(last["data"]["entries"].as_array().unwrap().len(), 5);
    let text = tool.execute(&json!({})).unwrap().message;
    assert!(text.contains("next offset=100"), "{text}");
    assert!(!text.contains("file-104"));
}

#[test]
fn serialized_size_limit_resumes_escaped_unicode_filenames_without_loss() {
    let workspace = TempDir::new().unwrap();
    let mut expected = Vec::new();
    for index in 0..400 {
        let suffix = if cfg!(windows) {
            "🌍".repeat(50)
        } else {
            "\"\t🌍".repeat(25)
        };
        let name = format!("{index:03}-{suffix}");
        fs::write(workspace.path().join(&name), "").unwrap();
        expected.push(name);
    }
    let tool = ListDirTool::new(workspace.path());
    let mut args = json!({"limit":1000,"format":"json"});
    let mut actual = Vec::new();
    loop {
        let result = page(&tool, args);
        for entry in result["data"]["entries"].as_array().unwrap() {
            actual.push(entry["path"].as_str().unwrap().to_string());
        }
        if !result["truncated"].as_bool().unwrap() {
            break;
        }
        assert!(!result["data"]["entries"].as_array().unwrap().is_empty());
        args = result["next_cursor"].clone();
    }
    assert_eq!(actual, expected);
}

#[test]
fn empty_directory_and_past_end_offset_have_no_next_page() {
    let workspace = TempDir::new().unwrap();
    let tool = ListDirTool::new(workspace.path());
    for offset in [0, 99] {
        let result = page(&tool, json!({"offset":offset,"format":"json"}));
        assert_eq!(result["data"]["entries"], json!([]));
        assert_eq!(result["truncated"], false);
        assert!(result["next_cursor"].is_null());
    }
}

#[test]
fn rejects_unsafe_paths_and_invalid_pagination() {
    let workspace = TempDir::new().unwrap();
    let tool = ListDirTool::new(workspace.path());
    for args in [
        json!({"path":"../"}),
        json!({"path":"/"}),
        json!({"path":""}),
        json!({"offset":-1}),
        json!({"offset":100001}),
        json!({"limit":0}),
        json!({"limit":1001}),
        json!({"limit":1.5}),
        json!({"format":"yaml"}),
        json!({"limti":1}),
    ] {
        assert!(tool.execute(&args).is_err(), "accepted {args}");
    }
}

#[cfg(unix)]
#[test]
fn lists_symlinks_and_special_files_without_following_or_opening_them() {
    use std::{
        ffi::CString,
        os::unix::{ffi::OsStrExt, fs::symlink},
    };
    let workspace = TempDir::new().unwrap();
    let outside = TempDir::new().unwrap();
    fs::write(outside.path().join("secret"), "secret").unwrap();
    symlink(outside.path(), workspace.path().join("outside")).unwrap();
    let fifo = CString::new(workspace.path().join("pipe").as_os_str().as_bytes()).unwrap();
    assert_eq!(unsafe { libc::mkfifo(fifo.as_ptr(), 0o600) }, 0);
    let tool = ListDirTool::new(workspace.path());
    let result = page(&tool, json!({"format":"json"}));
    assert_eq!(
        result["data"]["entries"],
        json!([
            {"path":"outside","kind":"symlink"}, {"path":"pipe","kind":"other"}
        ])
    );
    assert!(tool
        .execute(&json!({"path":"outside","format":"json"}))
        .is_err());
}
