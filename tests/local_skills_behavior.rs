use atlas_agentic::skills::{local, SkillDraftInput, SkillScope, SkillStore};
use std::collections::BTreeMap;
fn draft() -> SkillDraftInput {
    SkillDraftInput {
        name: "local-demo".into(),
        title: Some("Local demo".into()),
        description: "Useful: local instructions".into(),
        instructions: "Read the guide.".into(),
        files: Some(BTreeMap::from([("refs/guide.txt".into(), "Guide".into())])),
    }
}
#[test]
fn create_edit_preserves_resources_and_checks_entire_package() {
    let t = tempfile::tempdir().unwrap();
    let store = SkillStore::new(t.path().join("personal"), t.path().join("workspace"));
    let info = store.create(draft(), SkillScope::Personal).unwrap();
    assert!(info.source.is_none());
    assert!(store.create(draft(), SkillScope::Personal).is_err());
    store.set_enabled(&info.id, false).unwrap();
    let doc = store.read(&info.id, None).unwrap();
    std::fs::write(
        t.path().join("personal/local-demo/refs/guide.txt"),
        "Changed",
    )
    .unwrap();
    assert!(store
        .update_document(&info.id, &doc.content, &doc.fingerprint)
        .is_err());
    let doc = store.read(&info.id, None).unwrap();
    let updated = store
        .update_document(
            &info.id,
            &doc.content.replace("Read the guide.", "Read carefully."),
            &doc.fingerprint,
        )
        .unwrap();
    assert!(!updated.enabled);
    assert_eq!(
        store
            .read(&info.id, Some("refs/guide.txt"))
            .unwrap()
            .content,
        "Changed"
    );
    assert!(store
        .update_document(
            &info.id,
            &doc.content.replace("local-demo", "renamed"),
            &doc.fingerprint
        )
        .is_err());
}
#[test]
fn local_folder_and_individual_file_have_distinct_imports_and_detect_changes() {
    let t = tempfile::tempdir().unwrap();
    let store = SkillStore::new(t.path().join("personal"), t.path().join("workspace"));
    store.create(draft(), SkillScope::Personal).unwrap();
    let folder = t.path().join("personal/local-demo");
    let preview = local::preview(&folder).unwrap().remove(0);
    assert!(preview.files.contains(&"refs/guide.txt".into()));
    assert!(local::download(&folder, &preview.fingerprint)
        .unwrap()
        .source
        .is_none());
    let file_preview = local::preview(&folder.join("SKILL.md")).unwrap().remove(0);
    assert_eq!(file_preview.files, vec!["SKILL.md"]);
    std::fs::write(folder.join("refs/guide.txt"), "Changed").unwrap();
    assert!(local::download(&folder, &preview.fingerprint).is_err());
    assert!(local::download(&folder.join("SKILL.md"), &file_preview.fingerprint).is_ok());
}
#[test]
fn invalid_drafts_and_collection_imports_are_bounded() {
    let mut input = draft();
    input
        .files
        .as_mut()
        .unwrap()
        .insert("SKILL.md".into(), "override".into());
    assert!(input.validate().is_err());
    input.files = Some(BTreeMap::from([("../escape".into(), "bad".into())]));
    assert!(input.validate().is_err());
    input.files = None;
    input.instructions.clear();
    assert!(input.validate().is_err());
    let t = tempfile::tempdir().unwrap();
    let store = SkillStore::new(t.path().join("collection"), t.path().join("workspace"));
    store.create(draft(), SkillScope::Personal).unwrap();
    assert_eq!(
        local::preview(&t.path().join("collection")).unwrap().len(),
        1
    );
    let custom = atlas_agentic::settings::SettingsStore::at(t.path().join("custom/settings.json"));
    let isolated = SkillStore::for_settings(&custom, t.path().join("workspace")).unwrap();
    isolated.create(draft(), SkillScope::Personal).unwrap();
    assert!(t.path().join("custom/skills/local-demo/SKILL.md").is_file());
}
#[test]
fn universal_root_resolution_is_platform_independent() {
    use atlas_agentic::skills::shared_skills_root;
    use std::path::Path;
    let t = tempfile::tempdir().unwrap();
    let config = t.path().join("config");
    let home = t.path().join("home");
    assert_eq!(
        shared_skills_root(Some(&config), &home),
        config.join("agents/skills")
    );
    assert_eq!(
        shared_skills_root(Some(Path::new("relative-config")), &home),
        home.join(".config/agents/skills")
    );
    assert_eq!(
        shared_skills_root(None, &home),
        home.join(".config/agents/skills")
    );
}

#[cfg(unix)]
#[test]
fn local_import_rejects_symlinks_and_special_files() {
    use std::os::unix::fs::symlink;
    let t = tempfile::tempdir().unwrap();
    let store = SkillStore::new(t.path().join("personal"), t.path().join("workspace"));
    store.create(draft(), SkillScope::Personal).unwrap();
    let path = t.path().join("personal/local-demo");
    symlink(path.join("SKILL.md"), path.join("refs/link")).unwrap();
    assert!(local::preview(&path).is_err());
    std::fs::remove_file(path.join("refs/link")).unwrap();
    symlink(&path, t.path().join("link")).unwrap();
    assert!(local::preview(&t.path().join("link")).is_err());
}
