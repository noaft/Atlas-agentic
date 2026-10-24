use atlas_agentic::skills::{parse_skill, SkillPackage, SkillScope, SkillSource, SkillStore};
use atlas_agentic::tools::{LoadSkillTool, Tool};
use serde_json::json;
use std::collections::BTreeMap;

fn source() -> SkillSource {
    SkillSource {
        repository: "owner/repo".into(),
        reference: "main".into(),
        revision: "a".repeat(40),
        path: "skills/demo".into(),
    }
}
fn package(body: &str) -> SkillPackage {
    SkillPackage {
        source: source(),
        files: BTreeMap::from([
            (
                "SKILL.md".into(),
                format!("---\nname: demo\ndescription: Helps with demos\n---\n{body}").into_bytes(),
            ),
            ("refs/guide.md".into(), b"Supporting text".to_vec()),
        ]),
    }
}
fn store(t: &tempfile::TempDir) -> SkillStore {
    SkillStore::new(t.path().join("personal"), t.path().join("workspace"))
}
#[test]
fn validates_frontmatter_and_preserves_full_text() {
    assert_eq!(
        parse_skill(
            "---\nname: demo\ndescription: Fine\nmetadata:\n  title: Demo title\n---\nbody"
        )
        .unwrap()
        .title,
        "Demo title"
    );
    for text in [
        "no yaml",
        "---\nname: ../demo\ndescription: a\n---",
        "---\nname: demo\ndescription: [a]\n---",
        "---\nname: demo\nname: other\ndescription: a\n---",
    ] {
        assert!(parse_skill(text).is_err(), "{text}");
    }
    assert!(parse_skill(&format!(
        "---\nname: demo\ndescription: {}\n---",
        "a".repeat(1025)
    ))
    .is_err());
}
#[test]
fn scoped_catalog_disable_updates_and_lazy_tool_snapshot() {
    let t = tempfile::tempdir().unwrap();
    let s = store(&t);
    let info = s
        .install(
            package("Secret full instructions"),
            SkillScope::Personal,
            false,
        )
        .unwrap();
    assert_eq!(info.id, "personal:demo");
    s.install(package("project"), SkillScope::Project, false)
        .unwrap();
    assert_eq!(s.list().unwrap().skills.len(), 2);
    assert!(s
        .install(package("duplicate"), SkillScope::Personal, false)
        .is_err());
    let tool = LoadSkillTool::new(&s).unwrap().unwrap();
    assert!(!tool.definition().description.contains("Secret full"));
    assert!(tool
        .execute(&json!({"id":"personal:demo"}))
        .unwrap()
        .message
        .contains("Secret full instructions"));
    assert_eq!(
        tool.execute(&json!({"id":"personal:demo","resource":"refs/guide.md"}))
            .unwrap()
            .message,
        "Supporting text"
    );
    assert!(tool
        .execute(&json!({"id":"personal:demo","resource":"../SKILL.md"}))
        .is_err());
    s.set_enabled("personal:demo", false).unwrap();
    s.install(package("updated"), SkillScope::Personal, true)
        .unwrap();
    assert!(!s.read("personal:demo", None).unwrap().skill.enabled);
    let new = LoadSkillTool::new(&s).unwrap().unwrap();
    assert!(new.execute(&json!({"id":"personal:demo"})).is_err());
    // Running requests retain their immutable original content.
    assert!(tool
        .execute(&json!({"id":"personal:demo"}))
        .unwrap()
        .message
        .contains("Secret full instructions"));
}
#[test]
fn unsafe_or_invalid_packages_never_replace_existing_skill() {
    let t = tempfile::tempdir().unwrap();
    let s = store(&t);
    s.install(package("original"), SkillScope::Personal, false)
        .unwrap();
    for path in [
        "../escape",
        "/absolute",
        "refs/../bad",
        ".atlas-skill.json",
        "refs\\bad",
        "refs//bad",
    ] {
        let mut p = package("bad");
        p.files.insert(path.into(), vec![1]);
        assert!(s.install(p, SkillScope::Personal, true).is_err(), "{path}");
    }
    let mut p = package("bad");
    p.source.revision = "main".into();
    assert!(s.install(p, SkillScope::Personal, true).is_err());
    let mut p = package("bad");
    p.files.insert("refs/huge".into(), vec![0; 256 * 1024 + 1]);
    assert!(s.install(p, SkillScope::Personal, true).is_err());
    assert!(s
        .read("personal:demo", None)
        .unwrap()
        .content
        .ends_with("original"));
}
#[test]
fn malformed_skills_are_reported_and_binary_resources_actionable() {
    let t = tempfile::tempdir().unwrap();
    let s = store(&t);
    let mut p = package("body");
    p.files.insert("image.png".into(), vec![255]);
    s.install(p, SkillScope::Personal, false).unwrap();
    assert!(s
        .read("personal:demo", Some("image.png"))
        .unwrap_err()
        .contains("UTF-8"));
    std::fs::create_dir_all(t.path().join("personal/broken")).unwrap();
    std::fs::write(t.path().join("personal/broken/SKILL.md"), "bad").unwrap();
    assert_eq!(s.list().unwrap().skills.len(), 1);
    assert!(!s.list().unwrap().issues.is_empty());
}
#[cfg(unix)]
#[test]
fn symlink_roots_and_resources_are_rejected() {
    use std::os::unix::fs::symlink;
    let t = tempfile::tempdir().unwrap();
    let s = store(&t);
    s.install(package("body"), SkillScope::Personal, false)
        .unwrap();
    symlink(t.path(), t.path().join("personal/demo/escape")).unwrap();
    assert!(s.read("personal:demo", Some("escape/anything")).is_err());
    let t2 = tempfile::tempdir().unwrap();
    symlink(t.path().join("personal"), t2.path().join("personal")).unwrap();
    assert!(store(&t2)
        .install(package("bad"), SkillScope::Personal, false)
        .is_err());
}
#[test]
fn catalog_limits_and_non_string_metadata_are_rejected() {
    assert!(parse_skill("---\nname: demo\ndescription: 123\n---\nbody").is_err());
    assert!(parse_skill(&format!(
        "---\nname: demo\ndescription: Fine\n---\n{}",
        "a".repeat(128 * 1024)
    ))
    .is_err());
    let t = tempfile::tempdir().unwrap();
    let s = store(&t);
    let mut p = package("body");
    for n in 0..256 {
        p.files.insert(format!("extra/{n}"), vec![]);
    }
    assert!(s.install(p, SkillScope::Personal, false).is_err());
    let mut p = package("body");
    p.files
        .insert("refs".into(), b"file directory collision".to_vec());
    assert!(s.install(p, SkillScope::Personal, false).is_err());
    assert!(s.list().unwrap().skills.is_empty());
    assert!(LoadSkillTool::new(&s).unwrap().is_none());
}
#[test]
fn runtime_catalog_is_bounded_and_contains_metadata_only() {
    let t = tempfile::tempdir().unwrap();
    let s = store(&t);
    for n in 0..65 {
        let mut p = package("Instructions never in catalog");
        p.files.insert(
            "SKILL.md".into(),
            format!(
                "---\nname: skill-{n}\ndescription: A helper\n---\nInstructions never in catalog"
            )
            .into_bytes(),
        );
        s.install(p, SkillScope::Personal, false).unwrap();
    }
    assert!(s
        .list()
        .unwrap()
        .issues
        .iter()
        .any(|i| i.contains("64 enabled")));
    let tool = LoadSkillTool::new(&s).unwrap().unwrap();
    let definition = tool.definition();
    assert_eq!(
        definition.input_schema["properties"]["id"]["enum"]
            .as_array()
            .unwrap()
            .len(),
        64
    );
    assert!(!definition.description.contains("revision"));
    assert!(!definition.description.contains("Instructions never"));
}
#[test]
fn platform_ambiguous_resource_paths_are_rejected() {
    let t = tempfile::tempdir().unwrap();
    let s = store(&t);
    for name in [
        "refs/.. /escape",
        "refs/CON.txt",
        "refs/trailing.",
        "refs/trailing ",
    ] {
        let mut p = package("body");
        p.files.insert(name.into(), vec![]);
        assert!(s.install(p, SkillScope::Personal, false).is_err(), "{name}");
    }
}
#[test]
fn interrupted_update_recovers_previous_package_and_enable_state() {
    let t = tempfile::tempdir().unwrap();
    let s = store(&t);
    s.install(package("original"), SkillScope::Personal, false)
        .unwrap();
    s.set_enabled("personal:demo", false).unwrap();
    let backup = t
        .path()
        .join(format!("personal/.previous-{}", uuid::Uuid::new_v4()));
    std::fs::rename(t.path().join("personal/demo"), &backup).unwrap();
    let catalog = s.list().unwrap();
    assert_eq!(catalog.skills.len(), 1);
    assert!(!catalog.skills[0].enabled);
    assert!(!backup.exists());
    assert!(s
        .read("personal:demo", None)
        .unwrap()
        .content
        .ends_with("original"));
}
#[test]
fn packages_cannot_exceed_readable_directory_bounds() {
    let t = tempfile::tempdir().unwrap();
    let s = store(&t);
    let mut deep = package("body");
    deep.files
        .insert(format!("{}file", "dir/".repeat(33)), vec![]);
    assert!(s.install(deep, SkillScope::Personal, false).is_err());
    let mut broad = package("body");
    for n in 0..129 {
        broad
            .files
            .insert(format!("branch-{n}/a/b/c/d/e/f/g/h/file"), vec![]);
    }
    assert!(s.install(broad, SkillScope::Personal, false).is_err());
}
#[test]
fn loaded_instructions_include_frozen_folder_and_all_relative_files_without_io() {
    let t = tempfile::tempdir().unwrap();
    let s = store(&t);
    let mut p = package("Use scripts/check.sh with refs/guide.md.");
    p.files
        .insert("scripts/check.sh".into(), b"echo check".to_vec());
    p.files.insert("assets/icon.bin".into(), vec![255]);
    s.install(p, SkillScope::Personal, false).unwrap();
    let original = s.read("personal:demo", None).unwrap().content;
    let directory = t.path().join("personal/demo");
    let tool = LoadSkillTool::new(&s).unwrap().unwrap();
    assert!(!tool
        .definition()
        .description
        .contains(directory.to_str().unwrap()));
    std::fs::remove_dir_all(&directory).unwrap();
    let result = tool
        .execute(&json!({"id":"personal:demo"}))
        .unwrap()
        .message;
    assert!(result.contains(&serde_json::to_string(&directory.to_string_lossy()).unwrap()));
    assert!(result.contains("scripts/check.sh"));
    assert!(result.contains("assets/icon.bin"));
    assert!(result.contains("relative to the skill folder"));
    assert!(result.contains("never runs scripts"));
    assert!(result.ends_with(&original));
    assert_eq!(
        tool.execute(&json!({"id":"personal:demo","resource":"refs/guide.md"}))
            .unwrap()
            .message,
        "Supporting text"
    );
}
