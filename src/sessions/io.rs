use serde::{de::DeserializeOwned, Serialize};
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Component, Path},
};
pub(super) fn options() -> OpenOptions {
    let mut options = OpenOptions::new();
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
    }
    options
}
pub(super) fn directory(root: &Path, relative: &Path) -> Result<std::path::PathBuf, String> {
    fs::create_dir_all(root).map_err(|_| "Could not create session data root")?;
    let mut path = root.to_path_buf();
    for component in relative.components() {
        let Component::Normal(part) = component else {
            return Err("Invalid session storage path".into());
        };
        path.push(part);
        match fs::symlink_metadata(&path) {
            Ok(meta) if !meta.is_dir() || meta.file_type().is_symlink() => {
                return Err("Unsafe session storage directory".into())
            }
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                let mut builder = fs::DirBuilder::new();
                #[cfg(unix)]
                {
                    use std::os::unix::fs::DirBuilderExt;
                    builder.mode(0o700);
                }
                if let Err(e) = builder.create(&path) {
                    if e.kind() != std::io::ErrorKind::AlreadyExists {
                        return Err("Could not create session directory".into());
                    }
                }
                let meta = fs::symlink_metadata(&path)
                    .map_err(|_| "Could not inspect session directory")?;
                if !meta.is_dir() || meta.file_type().is_symlink() {
                    return Err("Unsafe session storage directory".into());
                }
            }
            Err(_) => return Err("Could not inspect session directory".into()),
        }
    }
    Ok(path)
}
pub(super) fn read<T: DeserializeOwned>(path: &Path, limit: usize) -> Result<T, String> {
    let meta = fs::symlink_metadata(path).map_err(|_| "Could not inspect session JSON")?;
    if !meta.is_file() || meta.file_type().is_symlink() {
        return Err("Unsafe session JSON file".into());
    }
    let file = options()
        .read(true)
        .open(path)
        .map_err(|_| "Could not read session JSON")?;
    let mut bytes = Vec::new();
    file.take(limit as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "Could not read session JSON")?;
    if bytes.len() > limit {
        return Err("Session JSON exceeds configured size limit".into());
    }
    serde_json::from_slice(&bytes).map_err(|_| "Invalid session JSON or unsupported schema".into())
}
pub(super) fn write<T: Serialize>(path: &Path, value: &T, sync: bool) -> Result<(), String> {
    let bytes = serde_json::to_vec_pretty(value).map_err(|_| "Could not encode session JSON")?;
    let temporary = path.with_extension(format!("{}.tmp", uuid::Uuid::new_v4()));
    let result = (|| {
        let mut file = options()
            .create_new(true)
            .write(true)
            .open(&temporary)
            .map_err(|_| "Could not create session JSON")?;
        file.write_all(&bytes)
            .and_then(|()| file.write_all(b"\n"))
            .map_err(|_| "Could not write session JSON")?;
        if sync {
            file.sync_all().map_err(|_| "Could not sync session JSON")?;
        }
        drop(file);
        fs::rename(&temporary, path).map_err(|_| "Could not publish session JSON")?;
        #[cfg(unix)]
        if sync {
            File::open(path.parent().ok_or("Missing session directory")?)
                .and_then(|f| f.sync_all())
                .map_err(|_| "Could not sync session directory")?;
        }
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(temporary);
    }
    result
}
