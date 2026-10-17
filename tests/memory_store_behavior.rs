use atlas_agentic::conversations::*;
fn checkpoint(id: &ConversationId, messages: &[ConversationMessage]) -> MemoryCheckpoint {
    MemoryCheckpoint {
        version: 1,
        policy_version: 1,
        conversation_id: id.clone(),
        checkpoint_id: "one".into(),
        created_at: chrono::Utc::now(),
        model: "fake".into(),
        trigger: CompactionTrigger::Manual,
        occupancy_percent: 20.0,
        estimated: true,
        requested_blocks: 1,
        compacted_message_count: 2,
        prefix_fingerprint: memory_fingerprint(&messages[..2]).unwrap(),
        blocks: vec![MemoryBlock {
            start_index: 0,
            end_index_exclusive: 2,
            content: "remember".into(),
        }],
        summary_usage: vec![],
    }
}
#[test]
fn checkpoint_is_loaded_replaced_atomically_and_moves_with_history() {
    let dir = tempfile::tempdir().unwrap();
    let store = ConversationStore::at(dir.path().into());
    let meta = store.create_turn("first", "answer").unwrap();
    assert!(store.load_memory(&meta.id).unwrap().is_none());
    let cp = checkpoint(&meta.id, &store.load(&meta.id).unwrap().messages);
    store.replace_memory(&meta.id, None, &cp).unwrap();
    assert_eq!(store.load_memory(&meta.id).unwrap(), Some(cp.clone()));
    assert!(store.replace_memory(&meta.id, None, &cp).is_err());
    let mut invalid = cp.clone();
    invalid.checkpoint_id = "two".into();
    invalid.blocks[0].start_index = 1;
    assert!(store
        .replace_memory(&meta.id, Some("one"), &invalid)
        .is_err());
    assert_eq!(store.load_memory(&meta.id).unwrap(), Some(cp));
    store.append_turn(&meta.id, "next", "answer").unwrap();
    assert!(store.load_memory(&meta.id).unwrap().is_some());
    store.rename(&meta.id, "renamed").unwrap();
    store.delete(&meta.id).unwrap();
    assert!(dir
        .path()
        .join(format!("trash/{}.memory.json", meta.id))
        .exists());
}
#[test]
fn execution_leases_are_per_conversation_and_release_on_drop() {
    let dir = tempfile::tempdir().unwrap();
    let store = ConversationStore::at(dir.path().into());
    let a = ConversationId::new();
    let b = ConversationId::new();
    let guard = store.acquire_execution(&a).unwrap();
    assert!(store.acquire_execution(&a).is_err());
    assert!(store.acquire_execution(&b).is_ok());
    drop(guard);
    assert!(store.acquire_execution(&a).is_ok());
}

#[test]
fn publisher_failure_keeps_checkpoint_and_original_transcript() {
    let dir = tempfile::tempdir().unwrap();
    let store = ConversationStore::at(dir.path().into());
    let meta = store.create_turn("first", "answer").unwrap();
    let cp = checkpoint(&meta.id, &store.load(&meta.id).unwrap().messages);
    store.replace_memory(&meta.id, None, &cp).unwrap();
    let failing = ConversationStore::at_with_index_publisher(dir.path().into(), |_, _| {
        Err(std::io::Error::other("injected publish failure"))
    });
    let mut next = cp.clone();
    next.checkpoint_id = "next".into();
    assert!(failing
        .replace_memory(&meta.id, Some("one"), &next)
        .is_err());
    assert_eq!(store.load_memory(&meta.id).unwrap(), Some(cp));
    assert!(failing.delete(&meta.id).is_err());
    assert!(store.load(&meta.id).unwrap().memory.is_some());
}
