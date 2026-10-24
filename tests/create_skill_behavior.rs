use std::{
    cell::{Cell, RefCell},
    collections::VecDeque,
};

use atlas_agentic::{
    agents::{AgentRegistry, LocalAgent, LocalAgentDefinition},
    models::{ModelProvider, ModelRequest},
    permissions::{
        AuthorizationDecision, AuthorizationRequest, PermissionMode, ToolAuthorizer, ToolRisk,
    },
    skills::SkillStore,
    tools::{CreateSkillTool, Tool, ToolRegistry},
    Runtime,
};
use serde_json::{json, Value};

fn draft(scope: &str) -> Value {
    json!({"scope":scope,"name":"team-guide","description":"Team workflows", "instructions":"Keep private instructions out of summaries.","files":{"references/guide.txt":"Support text"}})
}

struct QueueModel(RefCell<VecDeque<String>>);
impl ModelProvider for QueueModel {
    fn complete(&self, _: &ModelRequest) -> Result<String, String> {
        self.0
            .borrow_mut()
            .pop_front()
            .ok_or_else(|| "unexpected model call".into())
    }
}
struct Authorizer {
    calls: Cell<usize>,
    allow: bool,
}
impl ToolAuthorizer for Authorizer {
    fn authorize(&self, request: &AuthorizationRequest) -> Result<AuthorizationDecision, String> {
        self.calls.set(self.calls.get() + 1);
        assert!(!request.arguments.contains("Keep private"));
        assert!(!request.arguments.contains("Support text"));
        Ok(if self.allow {
            AuthorizationDecision::Allow
        } else {
            AuthorizationDecision::Deny
        })
    }
}
fn runtime(tool: CreateSkillTool, args: Value) -> Runtime {
    let definition = LocalAgentDefinition::from_toml(
        r#"
name = "atlas"
display_name = "Atlas"
model_provider = "fake"
model = "fake"
system_prompt = "You are Atlas."
conversation_title_prompt = "Write a title."
"#,
    )
    .unwrap();
    let responses = VecDeque::from([
        format!(r#"{{"type":"tool_call","tool":"create_skill","args":{args}}}"#),
        r#"{"type":"final","content":"Finished"}"#.to_string(),
    ]);
    Runtime::new(AgentRegistry::new(vec![Box::new(LocalAgent::new(
        definition,
        Box::new(QueueModel(RefCell::new(responses))),
    ))]))
    .with_tools(ToolRegistry::new(vec![Box::new(tool)]))
}

#[test]
fn runtime_enforces_scope_permissions_before_creating_files() {
    for (scope, mode, allow, expected_calls, exists) in [
        ("project", PermissionMode::Workspace, false, 0, true),
        ("personal", PermissionMode::Workspace, false, 1, false),
        ("project", PermissionMode::Ask, false, 1, false),
        ("personal", PermissionMode::Ask, true, 1, true),
        ("personal", PermissionMode::FullAccess, false, 0, true),
    ] {
        let root = tempfile::tempdir().unwrap();
        let personal = root.path().join("personal");
        let workspace = root.path().join("workspace");
        std::fs::create_dir(&workspace).unwrap();
        let tool = CreateSkillTool::new(SkillStore::new(personal.clone(), workspace.clone()));
        let authorizer = Authorizer {
            calls: Cell::new(0),
            allow,
        };
        let result = runtime(tool, draft(scope))
            .ask_stream_with_history_and_authorization(
                "Save this skill",
                &[],
                mode,
                &authorizer,
                &mut |_| {},
                &mut |_| {},
            )
            .unwrap();
        assert_eq!(result, "Finished");
        assert_eq!(authorizer.calls.get(), expected_calls);
        let target = if scope == "personal" {
            personal.clone()
        } else {
            workspace.join(".agents/skills")
        };
        assert_eq!(target.join("team-guide/SKILL.md").exists(), exists);
        if !exists {
            assert!(
                !target.exists(),
                "denied tools must not create even parent directories"
            );
        }
    }
}

#[test]
fn tool_returns_metadata_and_never_overwrites() {
    let root = tempfile::tempdir().unwrap();
    let tool = CreateSkillTool::new(SkillStore::new(
        root.path().join("personal"),
        root.path().to_path_buf(),
    ));
    assert_eq!(
        tool.risk(&draft("project")).unwrap(),
        ToolRisk::ModifyWorkspace
    );
    assert_eq!(
        tool.risk(&draft("personal")).unwrap(),
        ToolRisk::ModifyOutsideWorkspace
    );
    assert!(!root.path().join("personal").exists());
    let result = tool.execute(&draft("project")).unwrap();
    let info: Value = serde_json::from_str(&result.message).unwrap();
    assert_eq!(info["id"], "project:team-guide");
    assert_eq!(info["source"], Value::Null);
    assert!(!result.message.contains("Keep private"));
    assert!(tool.execute(&draft("project")).is_err());
    assert_eq!(
        std::fs::read_to_string(
            root.path()
                .join(".agents/skills/team-guide/references/guide.txt")
        )
        .unwrap(),
        "Support text"
    );
}

#[test]
fn invalid_drafts_fail_before_authorization_without_writes() {
    let root = tempfile::tempdir().unwrap();
    let tool = CreateSkillTool::new(SkillStore::new(
        root.path().join("personal"),
        root.path().join("workspace"),
    ));
    for (key, value) in [
        ("name", json!("../escape")),
        ("scope", json!("outside")),
        ("instructions", json!("")),
        ("files", json!({"../escape.txt":"bad"})),
        ("files", json!({"SKILL.md":"override"})),
        ("extra", json!(true)),
    ] {
        let mut args = draft("project");
        args[key] = value;
        assert!(tool.risk(&args).is_err(), "accepted invalid {key}: {args}");
    }
    assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 0);
}

#[test]
fn newly_created_skills_enter_the_next_load_snapshot() {
    use atlas_agentic::tools::LoadSkillTool;
    let root = tempfile::tempdir().unwrap();
    let store = SkillStore::new(root.path().join("personal"), root.path().to_path_buf());
    assert!(LoadSkillTool::new(&store).unwrap().is_none());
    CreateSkillTool::new(store.clone())
        .execute(&draft("project"))
        .unwrap();
    let loader = LoadSkillTool::new(&store).unwrap().unwrap();
    let result = loader.execute(&json!({"id":"project:team-guide"})).unwrap();
    assert!(result.message.contains("Keep private instructions"));
}

#[cfg(unix)]
#[test]
fn project_root_symlinks_cannot_redirect_workspace_authorized_writes() {
    let root = tempfile::tempdir().unwrap();
    let workspace = root.path().join("workspace");
    let outside = root.path().join("outside");
    std::fs::create_dir(&workspace).unwrap();
    std::fs::create_dir(&outside).unwrap();
    std::os::unix::fs::symlink(&outside, workspace.join(".agents")).unwrap();
    let tool = CreateSkillTool::new(SkillStore::new(root.path().join("personal"), workspace));
    assert!(tool.execute(&draft("project")).is_err());
    assert_eq!(std::fs::read_dir(outside).unwrap().count(), 0);
}
