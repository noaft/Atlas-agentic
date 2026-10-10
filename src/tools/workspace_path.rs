use std::path::{Component, Path, PathBuf};

use cap_std::{ambient_authority, fs::Dir};

pub fn validate_relative_path(tool: &str, field: &str, value: &str) -> Result<PathBuf, String> {
    let path = Path::new(value);
    if path.as_os_str().is_empty() {
        return Err(format!("{tool} {field} is required"));
    }
    if path.is_absolute() {
        return Err(format!("{tool} {field} must be relative"));
    }
    if path
        .components()
        .any(|component| matches!(component, Component::ParentDir))
    {
        return Err(format!(
            "{tool} {field} cannot contain parent directory segments"
        ));
    }
    Ok(path.to_path_buf())
}

pub fn open_workspace(tool: &str, workspace: &Path) -> Result<Dir, String> {
    Dir::open_ambient_dir(workspace, ambient_authority())
        .map_err(|error| format!("failed to open {tool} workspace: {error}"))
}

pub fn require_regular_file(
    tool: &str,
    field: &str,
    workspace: &Dir,
    path: &Path,
) -> Result<(), String> {
    let metadata = workspace
        .symlink_metadata(path)
        .map_err(|error| format!("failed to inspect {tool} {field}: {error}"))?;
    if !metadata.is_file() {
        return Err(format!("{tool} {field} is not a file: {}", path.display()));
    }
    Ok(())
}
