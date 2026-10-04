use atlas_agentic::{
    conversations::{ConversationId, ConversationStore},
    handle_command, render_conversation_metadata, AppResult,
};

const ID: &str = "c93c7717-7522-47d4-a53c-7f92c62f66b5";

#[test]
fn shows_help_when_no_command_is_given() {
    let args: Vec<&str> = vec![];
    let result = handle_command(args);

    assert_eq!(result.unwrap(), AppResult::Help);
}

#[test]
fn ask_command_uses_streaming_by_default() {
    let result = handle_command(["ask", "hello atlas"]);

    assert_eq!(
        result.unwrap(),
        AppResult::Ask {
            prompt: "hello atlas".to_string(),
            conversation_id: None,
        }
    );
}

#[test]
fn ask_stream_command_returns_stream_request() {
    let result = handle_command(["ask", "--stream", "hello atlas"]);

    assert_eq!(
        result.unwrap(),
        AppResult::Ask {
            prompt: "hello atlas".to_string(),
            conversation_id: None,
        }
    );
}

#[test]
fn ask_accepts_conversation_and_stream_options_in_either_order() {
    for args in [
        vec!["ask", "--conversation", ID, "hello", "atlas"],
        vec!["ask", "--stream", "--conversation", ID, "hello", "atlas"],
        vec!["ask", "--conversation", ID, "--stream", "hello", "atlas"],
    ] {
        assert_eq!(
            handle_command(args).unwrap(),
            AppResult::Ask {
                prompt: "hello atlas".to_string(),
                conversation_id: Some(ConversationId::parse(ID).unwrap()),
            }
        );
    }
}

#[test]
fn ask_preserves_flags_after_prompt_as_prompt_text() {
    assert_eq!(
        handle_command(["ask", "explain", "--conversation", "--unknown"]).unwrap(),
        AppResult::Ask {
            prompt: "explain --conversation --unknown".to_string(),
            conversation_id: None,
        }
    );
}

#[test]
fn parses_conversation_management_commands() {
    let id = ConversationId::parse(ID).unwrap();
    for (args, expected) in [
        (vec!["conversation", "list"], AppResult::ConversationList),
        (
            vec!["conversation", "show", ID],
            AppResult::ConversationShow { id: id.clone() },
        ),
        (
            vec!["conversation", "rename", ID, "Atlas runtime work"],
            AppResult::ConversationRename {
                id: id.clone(),
                title: "Atlas runtime work".to_string(),
            },
        ),
        (
            vec!["conversation", "delete", ID],
            AppResult::ConversationDelete { id },
        ),
    ] {
        assert_eq!(handle_command(args).unwrap(), expected);
    }
}

#[test]
fn rejects_incomplete_or_invalid_ask_arguments() {
    for (args, expected) in [
        (vec!["ask"], "ask needs a prompt"),
        (vec!["ask", "--stream"], "ask needs a prompt"),
        (vec!["ask", "   "], "ask needs a prompt"),
        (vec!["ask", "--conversation"], "--conversation needs an ID"),
        (
            vec!["ask", "--conversation", "--stream"],
            "--conversation needs an ID",
        ),
        (vec!["ask", "--conversation", ID], "ask needs a prompt"),
        (
            vec!["ask", "--conversation", "invalid", "hello"],
            "invalid conversation ID",
        ),
        (
            vec!["ask", "--unknown", "hello"],
            "unknown ask option: --unknown",
        ),
        (
            vec!["ask", "--conversation", ID, "--conversation", ID, "hello"],
            "--conversation can only be supplied once",
        ),
    ] {
        let error = handle_command(args.clone()).unwrap_err();
        assert!(error.contains(expected), "{args:?}: {error}");
    }
}

#[test]
fn rejects_incomplete_invalid_and_extra_management_arguments() {
    for (args, expected) in [
        (vec!["conversation"], "conversation needs a command"),
        (
            vec!["conversation", "unknown"],
            "unknown conversation command",
        ),
        (vec!["conversation", "list", "extra"], "unexpected argument"),
        (vec!["conversation", "show"], "show needs an ID"),
        (vec!["conversation", "rename"], "rename needs an ID"),
        (vec!["conversation", "delete"], "delete needs an ID"),
        (
            vec!["conversation", "show", "invalid"],
            "invalid conversation ID",
        ),
        (
            vec!["conversation", "rename", "invalid", "title"],
            "invalid conversation ID",
        ),
        (
            vec!["conversation", "delete", "invalid"],
            "invalid conversation ID",
        ),
        (vec!["conversation", "rename", ID], "rename needs a title"),
        (
            vec!["conversation", "rename", ID, "   "],
            "rename needs a title",
        ),
        (
            vec!["conversation", "show", ID, "extra"],
            "unexpected argument",
        ),
        (
            vec!["conversation", "rename", ID, "title", "extra"],
            "unexpected argument",
        ),
        (
            vec!["conversation", "delete", ID, "extra"],
            "unexpected argument",
        ),
    ] {
        let error = handle_command(args.clone()).unwrap_err();
        assert!(error.contains(expected), "{args:?}: {error}");
    }
}

#[test]
fn metadata_rendering_keeps_manual_titles_on_one_line() {
    let data = tempfile::tempdir().unwrap();
    let store = ConversationStore::at(data.path().to_path_buf());
    let saved = store.create_turn("hello", "answer").unwrap();
    let renamed = store
        .rename(&saved.id, "First line\r\nSecond line")
        .unwrap();

    assert_eq!(
        render_conversation_metadata(&renamed),
        format!("Conversation: {} | First line Second line", saved.id)
    );
    assert_eq!(
        store.load(&saved.id).unwrap().metadata.title,
        "First line\r\nSecond line"
    );
}
