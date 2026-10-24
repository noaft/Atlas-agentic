//! Local, bounded skill packages. Instructions are loaded only on request.
pub mod github;
mod store;
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, path::PathBuf};
pub use store::SkillStore;

pub const MAX_FILES: usize = 256;
pub const MAX_FILE_BYTES: usize = 256 * 1024;
pub const MAX_PACKAGE_BYTES: usize = 8 * 1024 * 1024;
pub const MAX_SKILL_BYTES: usize = 128 * 1024;
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SkillScope {
    Personal,
    Project,
}
impl SkillScope {
    pub fn label(self) -> &'static str {
        match self {
            Self::Personal => "personal",
            Self::Project => "project",
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillSource {
    pub repository: String,
    pub reference: String,
    pub revision: String,
    pub path: String,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillInfo {
    pub id: String,
    pub name: String,
    pub title: String,
    pub description: String,
    pub scope: SkillScope,
    pub enabled: bool,
    pub source: Option<SkillSource>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillCatalog {
    pub skills: Vec<SkillInfo>,
    pub issues: Vec<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillDocument {
    pub skill: SkillInfo,
    pub content: String,
    pub files: Vec<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillPreview {
    pub source: SkillSource,
    pub name: String,
    pub title: String,
    pub description: String,
    pub content: String,
    pub files: Vec<String>,
}
#[derive(Debug, Clone)]
pub struct SkillPackage {
    pub source: SkillSource,
    pub files: BTreeMap<String, Vec<u8>>,
}
#[derive(Debug, Clone, Deserialize)]
pub struct SkillMetadata {
    pub name: String,
    pub title: String,
    pub description: String,
}
#[derive(Deserialize)]
struct Frontmatter {
    name: String,
    description: String,
    title: Option<String>,
    metadata: Option<Metadata>,
}
#[derive(Deserialize)]
struct Metadata {
    title: Option<String>,
}
pub fn parse_skill(content: &str) -> Result<SkillMetadata, String> {
    if content.len() > MAX_SKILL_BYTES {
        return Err("SKILL.md exceeds 128 KiB".into());
    }
    let normalized = content.strip_prefix('\u{feff}').unwrap_or(content);
    let mut lines = normalized.lines();
    if lines.next() != Some("---") {
        return Err("SKILL.md requires YAML frontmatter".into());
    }
    let mut yaml = String::new();
    let mut closed = false;
    for line in lines {
        if line == "---" {
            closed = true;
            break;
        }
        yaml.push_str(line);
        yaml.push('\n');
        if yaml.len() > 16 * 1024 {
            return Err("skill frontmatter exceeds 16 KiB".into());
        }
    }
    if !closed {
        return Err("skill frontmatter is not closed".into());
    }
    let value: serde_yaml::Value =
        serde_yaml::from_str(&yaml).map_err(|_| "invalid skill YAML metadata".to_string())?;
    for field in ["name", "description", "title"] {
        if let Some(field_value) = value.get(field) {
            if !field_value.is_string() {
                return Err(format!("skill {field} must be a string"));
            }
        }
    }
    if let Some(title) = value.get("metadata").and_then(|v| v.get("title")) {
        if !title.is_string() {
            return Err("skill title must be a string".into());
        }
    }
    let f: Frontmatter =
        serde_yaml::from_value(value).map_err(|_| "invalid skill YAML metadata".to_string())?;
    validate_name(&f.name)?;
    let description = bounded_text(f.description, 1024, "description")?;
    let title = bounded_text(
        f.title
            .or_else(|| f.metadata.and_then(|m| m.title))
            .unwrap_or_else(|| f.name.clone()),
        160,
        "title",
    )?;
    Ok(SkillMetadata {
        name: f.name,
        title,
        description,
    })
}
fn bounded_text(value: String, max: usize, field: &str) -> Result<String, String> {
    let value = value.trim();
    if value.is_empty()
        || value.chars().count() > max
        || value
            .chars()
            .any(|c| c.is_control() && !matches!(c, '\n' | '\t'))
    {
        return Err(format!("invalid skill {field}"));
    }
    Ok(value.into())
}
pub(crate) fn validate_name(name: &str) -> Result<(), String> {
    if name.is_empty()
        || name.len() > 64
        || name.starts_with('-')
        || name.ends_with('-')
        || name.contains("--")
        || !name
            .bytes()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'-')
    {
        return Err(
            "skill name must be lowercase letters, digits and single hyphens (1–64 bytes)".into(),
        );
    }
    Ok(())
}
pub(crate) fn validate_path(path: &str) -> Result<(), String> {
    if path.split('/').count() > 32
        || path.is_empty()
        || path.len() > 1024
        || path.contains('\\')
        || path.contains(':')
        || path.chars().any(char::is_control)
        || path.split('/').any(unsafe_component)
    {
        return Err("skill file path must be a safe relative path".into());
    }
    Ok(())
}
pub(crate) fn validate_source(source: &SkillSource) -> Result<(), String> {
    let parts = source.repository.split('/').collect::<Vec<_>>();
    if source.repository.len() > 200
        || parts.len() != 2
        || parts.iter().any(|p| {
            p.is_empty()
                || *p == "."
                || *p == ".."
                || !p
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b))
        })
        || source.reference.is_empty()
        || source.reference.len() > 256
        || source.reference.chars().any(char::is_control)
        || source.revision.len() != 40
        || !source.revision.bytes().all(|b| b.is_ascii_hexdigit())
    {
        return Err("invalid pinned skill source".into());
    }
    if !source.path.is_empty() {
        validate_path(&source.path)?;
    }
    Ok(())
}
pub fn personal_skills_root() -> Result<PathBuf, String> {
    Ok(crate::settings::SettingsStore::from_user_config()?
        .directory()
        .join("skills"))
}

fn unsafe_component(part: &str) -> bool {
    let basename = part.split('.').next().unwrap_or("").to_ascii_uppercase();
    part.is_empty()
        || part == "."
        || part == ".."
        || part.ends_with(['.', ' '])
        || matches!(basename.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || ["COM", "LPT"].iter().any(|prefix| {
            basename
                .strip_prefix(prefix)
                .is_some_and(|n| n.len() == 1 && matches!(n.as_bytes()[0], b'1'..=b'9'))
        })
}
pub(crate) fn model_metadata(skill: &SkillInfo) -> serde_json::Value {
    serde_json::json!({"id":skill.id,"name":skill.name,"title":skill.title,"description":skill.description})
}
