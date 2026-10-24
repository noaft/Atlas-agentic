//! Bounded local imports. Preview and install read bytes only; resources never execute.
use super::*;
use std::path::Path;

fn package(path: &Path) -> Result<SkillPackage, String> {
    let metadata = std::fs::symlink_metadata(path).map_err(|e| format!("local skill: {e}"))?;
    let mut files = BTreeMap::new();
    if metadata.is_file() {
        if path.file_name().and_then(|n| n.to_str()) != Some("SKILL.md") {
            return Err("select a SKILL.md file or skill folder".into());
        }
        let dir = store::open_root(path.parent().ok_or("invalid skill path")?, false)?;
        files.insert("SKILL.md".into(), store::read_bytes(&dir, "SKILL.md")?);
    } else if metadata.is_dir() {
        let dir = store::open_root(path, false)?;
        store::walk(&dir, "", &mut files, &mut 0, &mut 0)?;
        // Atlas state is local to the destination, never imported as package data.
        files.remove(".atlas-skill.json");
    } else {
        return Err("local skill cannot be a symlink or special file".into());
    }
    store::validate_files(&files)?;
    let text = std::str::from_utf8(files.get("SKILL.md").ok_or("folder is missing SKILL.md")?)
        .map_err(|_| "SKILL.md must be UTF-8")?;
    parse_skill(text)?;
    Ok(SkillPackage {
        source: None,
        files,
    })
}

fn collect(
    path: &Path,
    depth: usize,
    entries: &mut usize,
    candidates: &mut Vec<PathBuf>,
) -> Result<(), String> {
    let dir = store::open_root(path, false)?;
    if dir.symlink_metadata("SKILL.md").is_ok() {
        candidates.push(path.to_path_buf());
        if candidates.len() > 64 {
            return Err("local import exceeds 64 skill candidates".into());
        }
        return Ok(());
    }
    if depth >= 3 {
        return Ok(());
    }
    let mut children = Vec::new();
    for entry in dir.entries().map_err(|e| e.to_string())? {
        *entries += 1;
        if *entries > 1024 {
            return Err("local collection exceeds 1024 entries".into());
        }
        let entry = entry.map_err(|e| e.to_string())?;
        let metadata = dir
            .symlink_metadata(entry.file_name())
            .map_err(|e| e.to_string())?;
        if metadata.is_symlink() {
            return Err("local collection cannot contain symlinks".into());
        }
        if metadata.is_dir() {
            children.push(path.join(entry.file_name()));
        }
    }
    children.sort();
    for child in children {
        collect(&child, depth + 1, entries, candidates)?;
    }
    Ok(())
}

pub fn preview(path: &Path) -> Result<Vec<LocalSkillPreview>, String> {
    let metadata = std::fs::symlink_metadata(path).map_err(|e| e.to_string())?;
    let mut candidates = Vec::new();
    if metadata.is_file() {
        candidates.push(path.to_path_buf());
    } else if metadata.is_dir() {
        collect(path, 0, &mut 0, &mut candidates)?;
    } else {
        return Err("select a regular SKILL.md file or folder".into());
    }
    if candidates.is_empty() {
        return Err("no SKILL.md found within three folder levels".into());
    }
    let mut result = Vec::new();
    let mut total = 0;
    let mut names = std::collections::BTreeSet::new();
    for path in candidates {
        let package = package(&path)?;
        total += package.files.values().map(Vec::len).sum::<usize>();
        if total > MAX_PACKAGE_BYTES {
            return Err("local previews exceed 8 MiB; select a smaller folder".into());
        }
        let content = String::from_utf8(package.files["SKILL.md"].clone())
            .map_err(|_| "SKILL.md must be UTF-8")?;
        let metadata = parse_skill(&content)?;
        if !names.insert(metadata.name.clone()) {
            return Err("multiple selected skills use the same name".into());
        }
        result.push(LocalSkillPreview {
            name: metadata.name,
            title: metadata.title,
            description: metadata.description,
            content,
            files: package.files.keys().cloned().collect(),
            path: std::fs::canonicalize(path)
                .map_err(|e| e.to_string())?
                .to_str()
                .ok_or("local skill path must be UTF-8")?
                .into(),
            fingerprint: fingerprint(&package.files),
        });
    }
    Ok(result)
}

pub fn download(path: &Path, expected: &str) -> Result<SkillPackage, String> {
    let package = package(path)?;
    if fingerprint(&package.files) != expected {
        return Err("local skill changed since preview; preview it again".into());
    }
    Ok(package)
}
