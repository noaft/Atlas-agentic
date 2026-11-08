//! Exact single-file edits with an approval-bound preimage and atomic publication.
use super::{
    workspace_path::{open_workspace, validate_relative_path},
    Tool, ToolDefinition, ToolResult,
};
use crate::permissions::ToolRisk;
use cap_std::fs::{Dir, OpenOptions};
use fs2::FileExt;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::{
    any::Any,
    fs::File,
    io::{Read, Seek, Write},
    path::{Path, PathBuf},
    time::{Duration, Instant},
};
const MAX_BYTES: usize = 1024 * 1024;

pub struct ApplyPatchTool {
    workspace: PathBuf,
}
impl ApplyPatchTool {
    pub fn new(workspace: impl AsRef<Path>) -> Self {
        Self {
            workspace: workspace.as_ref().to_path_buf(),
        }
    }
    fn prepare(&self, args: &serde_json::Value) -> Result<Patch, String> {
        let input = parse(args)?;
        let path = validate_relative_path("apply_patch", "path", &input.path)?;
        let workspace = self
            .workspace
            .canonicalize()
            .map_err(|_| "Could not resolve patch workspace")?;
        let parent = resolve_parent(&workspace, &path)?;
        let name = path
            .file_name()
            .ok_or("Patch needs a file name")?
            .to_owned();
        let mut original = open_file(&parent, Path::new(&name))?;
        let before = read(&mut original)?;
        let mut after = before.clone();
        for edit in &input.edits {
            let first = after
                .find(&edit.old_text)
                .ok_or("old_text was not found; read the file again")?;
            let next = first
                + edit
                    .old_text
                    .chars()
                    .next()
                    .expect("nonempty edit")
                    .len_utf8();
            if after[next..].contains(&edit.old_text) {
                return Err("Each old_text must match exactly once; read the file again".into());
            }
            after = after.replacen(&edit.old_text, &edit.new_text, 1);
            if after.len() > MAX_BYTES {
                return Err("Patched file exceeds 1 MiB".into());
            }
        }
        Ok(Patch {
            workspace,
            parent,
            name,
            original,
            before,
            after,
            path: input.path,
            replacements: input.edits.len(),
        })
    }
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Input {
    path: String,
    edits: Vec<Edit>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Edit {
    old_text: String,
    new_text: String,
}
fn parse(args: &serde_json::Value) -> Result<Input, String> {
    let input: Input =
        serde_json::from_value(args.clone()).map_err(|_| "Invalid apply_patch arguments")?;
    validate_relative_path("apply_patch", "path", &input.path)?;
    if input.edits.is_empty()
        || input.edits.len() > 64
        || input.edits.iter().any(|e| e.old_text.is_empty())
        || input
            .edits
            .iter()
            .map(|e| e.old_text.len().saturating_add(e.new_text.len()))
            .sum::<usize>()
            > MAX_BYTES
    {
        return Err(
            "Provide 1–64 nonempty exact edits, with at most 1 MiB of replacement input".into(),
        );
    }
    Ok(input)
}
struct Patch {
    workspace: PathBuf,
    parent: Dir,
    name: std::ffi::OsString,
    original: File,
    before: String,
    after: String,
    path: String,
    replacements: usize,
}
fn resolve_parent(workspace: &Path, path: &Path) -> Result<Dir, String> {
    if workspace
        .canonicalize()
        .map_err(|_| "Patch workspace changed")?
        != workspace
    {
        return Err("Patch workspace changed; read the file again".into());
    }
    let mut parent = open_workspace("apply_patch", workspace)?;
    if let Some(ancestors) = path.parent() {
        for component in ancestors.components() {
            if component == std::path::Component::CurDir {
                continue;
            }
            let part = Path::new(component.as_os_str());
            let metadata = parent
                .symlink_metadata(part)
                .map_err(|_| "Could not inspect patch parent")?;
            if !metadata.is_dir() || metadata.file_type().is_symlink() {
                return Err("Patch parents must be directories, not symlinks".into());
            }
            parent = parent
                .open_dir(part)
                .map_err(|_| "Could not open patch parent")?;
        }
    }
    Ok(parent)
}
fn directory_identity(directory: &Dir) -> Result<same_file::Handle, String> {
    same_file::Handle::from_file(
        directory
            .try_clone()
            .map_err(|_| "Could not inspect patch parent identity")?
            .into_std_file(),
    )
    .map_err(|_| "Could not inspect patch parent identity".into())
}
fn options() -> OpenOptions {
    let mut options = OpenOptions::new();
    #[cfg(unix)]
    {
        use cap_std::fs::OpenOptionsExt;
        options
            .mode(0o600)
            .custom_flags(libc::O_NONBLOCK | libc::O_NOFOLLOW);
    }
    #[cfg(windows)]
    {
        use cap_std::fs::OpenOptionsExt;
        options.custom_flags(windows_sys::Win32::Storage::FileSystem::FILE_FLAG_OPEN_REPARSE_POINT);
    }
    options
}
fn open_file(parent: &Dir, path: &Path) -> Result<File, String> {
    let metadata = parent
        .symlink_metadata(path)
        .map_err(|_| "Could not inspect patch file")?;
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        return Err("Patch target must be a regular file, not a symlink".into());
    }
    let mut options = options();
    options.read(true);
    let file = parent
        .open_with(path, &options)
        .map_err(|_| "Could not open patch file")?
        .into_std();
    if !file
        .metadata()
        .map_err(|_| "Could not inspect opened patch file")?
        .is_file()
    {
        return Err("Patch target must be a regular file".into());
    }
    Ok(file)
}
fn read(file: &mut (impl Read + Seek)) -> Result<String, String> {
    file.rewind().map_err(|_| "Could not read patch file")?;
    let mut bytes = Vec::new();
    file.take((MAX_BYTES + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|_| "Could not read patch file")?;
    if bytes.len() > MAX_BYTES {
        return Err("Patch file exceeds 1 MiB".into());
    }
    String::from_utf8(bytes).map_err(|_| "Patch file must be UTF-8".into())
}
impl Patch {
    fn verify(&self) -> Result<(), String> {
        let parent = resolve_parent(&self.workspace, Path::new(&self.path))?;
        if directory_identity(&parent)? != directory_identity(&self.parent)? {
            return Err("Patch parent changed; read the file again".into());
        }
        let current = open_file(&parent, Path::new(&self.name))?;
        let previous = same_file::Handle::from_file(
            self.original
                .try_clone()
                .map_err(|_| "Could not inspect patch identity")?,
        )
        .map_err(|_| "Could not inspect patch identity")?;
        let handle = same_file::Handle::from_file(
            current
                .try_clone()
                .map_err(|_| "Could not inspect patch identity")?,
        )
        .map_err(|_| "Could not inspect patch identity")?;
        // Use the original handle for reads under its lock (Windows locks deny reads
        // through other independently opened handles, even in the same process).
        let mut original = &self.original;
        if previous != handle || read(&mut original)? != self.before {
            return Err("Patch file changed; read it again before editing".into());
        }
        Ok(())
    }
    fn publish(&self) -> Result<ToolResult, String> {
        let deadline = Instant::now() + Duration::from_millis(1500);
        loop {
            match self.original.try_lock_exclusive() {
                Ok(()) => break,
                Err(e)
                    if e.kind() == std::io::ErrorKind::WouldBlock && Instant::now() < deadline =>
                {
                    std::thread::sleep(Duration::from_millis(10))
                }
                Err(_) => return Err("Patch file is busy; retry after reading it again".into()),
            }
        }
        // A queued writer on an old inode cannot replace a newer published file.
        let result = (|| {
            self.verify()?;
            let temporary = format!(".atlas-patch-{}.tmp", uuid::Uuid::new_v4());
            let result = (|| {
                let mut options = options();
                options.write(true).create_new(true);
                let mut output = self
                    .parent
                    .open_with(&temporary, &options)
                    .map_err(|_| "Could not stage patch")?;
                output
                    .write_all(self.after.as_bytes())
                    .map_err(|_| "Could not write patch")?;
                #[cfg(unix)]
                {
                    use std::os::unix::fs::{MetadataExt, PermissionsExt};
                    let mode = self
                        .original
                        .metadata()
                        .map_err(|_| "Could not inspect patch permissions")?
                        .mode()
                        & 0o777;
                    output
                        .set_permissions(cap_std::fs::Permissions::from_std(
                            std::fs::Permissions::from_mode(mode),
                        ))
                        .map_err(|_| "Could not preserve patch permissions")?;
                }
                output.sync_all().map_err(|_| "Could not sync patch")?;
                drop(output);
                self.verify()?;
                self.parent
                    .rename(&temporary, &self.parent, &self.name)
                    .map_err(|_| "Could not publish patch")?;
                Ok(ToolResult::text(serde_json::json!({"path":self.path,"replacements":self.replacements,"before_sha256":format!("{:x}",Sha256::digest(self.before.as_bytes())),"after_sha256":format!("{:x}",Sha256::digest(self.after.as_bytes()))}).to_string()))
            })();
            let _ = self.parent.remove_file(&temporary);
            result
        })();
        let _ = FileExt::unlock(&self.original);
        result
    }
}
impl Tool for ApplyPatchTool {
    fn name(&self) -> &str {
        "apply_patch"
    }
    fn definition(&self) -> ToolDefinition {
        ToolDefinition{name:self.name().into(),description:"Edit one existing UTF-8 workspace file using sequential exact replacements. Each old_text must occur exactly once. All edits are checked before atomic publication; file changes during approval invalidate the patch. No create/delete/move. Max 64 edits and 1 MiB file/input.".into(),input_schema:serde_json::json!({"type":"object","required":["path","edits"],"additionalProperties":false,"properties":{"path":{"type":"string"},"edits":{"type":"array","minItems":1,"maxItems":64,"items":{"type":"object","required":["old_text","new_text"],"additionalProperties":false,"properties":{"old_text":{"type":"string","minLength":1},"new_text":{"type":"string"}}}}}})}
    }
    fn risk(&self, args: &serde_json::Value) -> Result<ToolRisk, String> {
        parse(args)?;
        Ok(ToolRisk::Destructive)
    }
    fn authorize(
        &self,
        args: &serde_json::Value,
    ) -> Result<(ToolRisk, Option<Box<dyn Any>>), String> {
        Ok((ToolRisk::Destructive, Some(Box::new(self.prepare(args)?))))
    }
    fn execute(&self, args: &serde_json::Value) -> Result<ToolResult, String> {
        self.prepare(args)?.publish()
    }
    fn execute_authorized(
        &self,
        _args: &serde_json::Value,
        context: Option<&dyn Any>,
    ) -> Result<ToolResult, String> {
        context
            .and_then(|value| value.downcast_ref::<Patch>())
            .ok_or_else(|| "Missing patch authorization snapshot".to_string())?
            .publish()
    }
}
