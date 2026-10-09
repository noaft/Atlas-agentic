use std::{
    fs,
    path::{Path, PathBuf},
};

use directories::BaseDirs;

use crate::conversations::validate_workspace;

pub const AUTOMATIC_WORKSPACE_DIR_NAME: &str = "Atlas";

pub fn automatic_workspace() -> Result<PathBuf, String> {
    let base_dirs = BaseDirs::new();
    provision_automatic_workspace(base_dirs.as_ref().map(BaseDirs::home_dir))
}

pub fn provision_automatic_workspace(home: Option<&Path>) -> Result<PathBuf, String> {
    let home = home.ok_or_else(|| "could not resolve the user home directory".to_string())?;
    let workspace = home.join(AUTOMATIC_WORKSPACE_DIR_NAME);
    fs::create_dir_all(&workspace).map_err(|error| {
        format!(
            "failed to create automatic Atlas workspace at {}: {error}",
            workspace.display()
        )
    })?;
    validate_workspace(&workspace)
}
