use std::fs;

use atlas_agentic::workspace::{provision_automatic_workspace, AUTOMATIC_WORKSPACE_DIR_NAME};

#[test]
fn provisions_and_canonicalizes_atlas_folder_inside_home() {
    let root = tempfile::tempdir().unwrap();
    let home = root.path().join("home");

    let workspace = provision_automatic_workspace(Some(&home)).unwrap();

    assert_eq!(AUTOMATIC_WORKSPACE_DIR_NAME, "Atlas");
    assert!(home.join("Atlas").is_dir());
    assert_eq!(workspace, home.join("Atlas").canonicalize().unwrap());
}

#[test]
fn rejects_an_unavailable_home_directory() {
    assert_eq!(
        provision_automatic_workspace(None).unwrap_err(),
        "could not resolve the user home directory"
    );
}

#[test]
fn rejects_a_home_path_that_is_not_a_directory() {
    let root = tempfile::tempdir().unwrap();
    let home = root.path().join("home-file");
    fs::write(&home, "not a directory").unwrap();

    let error = provision_automatic_workspace(Some(&home)).unwrap_err();

    assert!(error.contains("failed to create automatic Atlas workspace"));
    assert!(error.contains(&home.join("Atlas").display().to_string()));
}

#[cfg(unix)]
#[test]
fn rejects_a_non_unicode_automatic_workspace() {
    use std::{ffi::OsString, os::unix::ffi::OsStringExt};

    let root = tempfile::tempdir().unwrap();
    let home = root.path().join(OsString::from_vec(vec![b'h', 0x80]));

    let error = provision_automatic_workspace(Some(&home)).unwrap_err();

    assert_eq!(error, "workspace path must contain valid Unicode");
}
