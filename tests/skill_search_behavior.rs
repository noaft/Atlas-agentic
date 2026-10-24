use atlas_agentic::skills::{SkillDraftInput, SkillScope, SkillStore};

#[test]
fn bm25_ranks_rare_terms_searches_instructions_and_keeps_scopes_separate() {
    let t = tempfile::tempdir().unwrap();
    let store = SkillStore::new(t.path().join("personal"), t.path().join("project"));
    for (name, instructions, scope) in [
        ("review", "Review changes and code.", SkillScope::Personal),
        (
            "rust-tests",
            "Review Rust regression tests with cargo.",
            SkillScope::Personal,
        ),
        (
            "project-tests",
            "Rust regression tests.",
            SkillScope::Project,
        ),
    ] {
        store
            .create(
                SkillDraftInput {
                    name: name.into(),
                    title: None,
                    description: "Development workflow".into(),
                    instructions: instructions.into(),
                    files: None,
                },
                scope,
            )
            .unwrap();
    }
    let found = store
        .search("Rust regression", SkillScope::Personal)
        .unwrap();
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].skill.id, "personal:rust-tests");
    assert!(found[0].score.is_finite() && found[0].score > 0.0);
    let ranked = store
        .search("review regression", SkillScope::Personal)
        .unwrap();
    assert_eq!(ranked.len(), 2);
    assert_eq!(ranked[0].skill.name, "rust-tests");
    assert!(ranked[0].score > ranked[1].score);
    assert_eq!(
        store.search("RUST_TESTS", SkillScope::Personal).unwrap()[0]
            .skill
            .name,
        "rust-tests"
    );
    assert!(store
        .search("unrelated", SkillScope::Personal)
        .unwrap()
        .is_empty());
    assert_eq!(store.search("", SkillScope::Personal).unwrap().len(), 2);
    assert_eq!(
        store.search("regression", SkillScope::Project).unwrap()[0]
            .skill
            .id,
        "project:project-tests"
    );
    store.set_enabled("personal:rust-tests", false).unwrap();
    assert!(
        !store.search("cargo", SkillScope::Personal).unwrap()[0]
            .skill
            .enabled
    );
    assert!(store
        .search(&"a".repeat(513), SkillScope::Personal)
        .is_err());
}
