//! Preview and install public skill packages as data through runtime approvals.
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use super::{summarize_arguments, Tool, ToolDefinition, ToolRegistry, ToolResult};
use crate::{
    permissions::ToolRisk,
    skills::{self, github, SkillPackage, SkillScope, SkillStore},
};

const MAX_CANDIDATES: usize = 8;
const MAX_PREVIEW_BYTES: usize = 64 * 1024;

trait ImportSource: Send + Sync {
    fn fetch(&self, url: &str) -> Result<Vec<SkillPackage>, String>;
}
struct GithubSource;
impl ImportSource for GithubSource {
    fn fetch(&self, url: &str) -> Result<Vec<SkillPackage>, String> {
        let previews = github::preview(url)?;
        if previews.is_empty() || previews.len() > MAX_CANDIDATES {
            return Err(
                "Preview between 1 and 8 skills; use a specific GitHub skill folder URL.".into(),
            );
        }
        let mut packages = Vec::new();
        let mut bytes = 0;
        for preview in previews {
            let package = github::download(&preview.source)?;
            bytes += package.files.values().map(Vec::len).sum::<usize>();
            if bytes > skills::MAX_PACKAGE_BYTES {
                return Err("Skill preview exceeds 8 MiB; use a smaller skill folder.".into());
            }
            packages.push(package);
        }
        Ok(packages)
    }
}
struct Imports {
    store: SkillStore,
    packages: BTreeMap<String, SkillPackage>,
    epoch: u64,
}
struct PreviewSkillTool {
    imports: Arc<Mutex<Imports>>,
    source: Arc<dyn ImportSource>,
    epoch: Option<u64>,
}
struct InstallSkillTool {
    imports: Arc<Mutex<Imports>>,
}
fn tools(store: SkillStore, source: Box<dyn ImportSource>) -> (PreviewSkillTool, InstallSkillTool) {
    let imports = Arc::new(Mutex::new(Imports {
        store,
        packages: BTreeMap::new(),
        epoch: 0,
    }));
    (
        PreviewSkillTool {
            imports: imports.clone(),
            source: Arc::from(source),
            epoch: None,
        },
        InstallSkillTool { imports },
    )
}
/// Register a pair sharing exact reviewed bytes for this runtime, even with no installed skills.
pub fn register_skill_import_tools(registry: &mut ToolRegistry, store: SkillStore) {
    let (preview, install) = tools(store, Box::new(GithubSource));
    registry.register(Box::new(preview));
    registry.register(crate::executions::worker::tool(install));
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PreviewArgs {
    url: String,
}
impl PreviewArgs {
    fn parse(value: &Value) -> Result<Self, String> {
        let args: Self =
            serde_json::from_value(value.clone()).map_err(|_| "invalid preview_skill arguments")?;
        if args.url.len() > 2048 {
            return Err("Skill URL exceeds 2048 bytes.".into());
        }
        github::validate_url(&args.url)?;
        Ok(args)
    }
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct InstallArgs {
    token: String,
    scope: SkillScope,
}
impl InstallArgs {
    fn parse(value: &Value) -> Result<Self, String> {
        serde_json::from_value(value.clone()).map_err(|_| "invalid install_skill arguments".into())
    }
}
#[derive(Serialize)]
struct Preview {
    token: String,
    source: skills::SkillSource,
    name: String,
    title: String,
    description: String,
    content: String,
    files: Vec<String>,
    fingerprint: String,
}
fn package_preview(package: &SkillPackage) -> Result<Preview, String> {
    let source = package
        .source
        .clone()
        .ok_or("Imported skill is missing its pinned source.")?;
    skills::validate_source(&source)?;
    if package.files.len() > skills::MAX_FILES {
        return Err("Skill has too many files.".into());
    }
    for (path, bytes) in &package.files {
        skills::validate_path(path)?;
        if bytes.len() > skills::MAX_FILE_BYTES {
            return Err("Skill file exceeds 256 KiB.".into());
        }
    }
    let content = std::str::from_utf8(
        package
            .files
            .get("SKILL.md")
            .ok_or("Skill is missing SKILL.md.")?,
    )
    .map_err(|_| "SKILL.md must be UTF-8.")?;
    let metadata = skills::parse_skill(content)?;
    Ok(Preview {
        token: uuid::Uuid::new_v4().to_string(),
        source,
        name: metadata.name,
        title: metadata.title,
        description: metadata.description,
        content: content.into(),
        files: package.files.keys().cloned().collect(),
        fingerprint: skills::fingerprint(&package.files),
    })
}
impl Tool for PreviewSkillTool {
    fn prepare_execution(
        &self,
        args: Value,
        _context: Option<Box<dyn std::any::Any + Send>>,
    ) -> Result<crate::executions::worker::Work, String> {
        let epoch = self.imports.lock().unwrap_or_else(|e| e.into_inner()).epoch;
        let tool = Self {
            imports: self.imports.clone(),
            source: self.source.clone(),
            epoch: Some(epoch),
        };
        Ok(Box::new(move |_| tool.execute(&args)))
    }

    fn reset_request_state(&self) {
        let mut imports = self.imports.lock().unwrap_or_else(|e| e.into_inner());
        imports.epoch = imports.epoch.wrapping_add(1);
        imports.packages.clear();
    }
    fn name(&self) -> &str {
        "preview_skill"
    }
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: self.name().into(),
            description: "Preview public GitHub skills only when the user asks to find, download or install skills. Downloads data, never runs scripts or installs dependencies. Returns full instructions, file names, pinned source and a token bound to the exact downloaded package. Review the instructions before install_skill. Tokens last only for this request; a new successful preview replaces old tokens. Use a specific skill folder for large repositories. Never install proactively; preserve the user's requested scope.".into(),
            input_schema: json!({"type":"object","required":["url"],"additionalProperties":false,"properties":{"url":{"type":"string","maxLength":2048,"description":"Public https://github.com repository, skill folder, or SKILL.md URL."}}}),
        }
    }
    fn risk(&self, args: &Value) -> Result<ToolRisk, String> {
        PreviewArgs::parse(args)?;
        Ok(ToolRisk::ExternalTool)
    }
    fn execute(&self, args: &Value) -> Result<ToolResult, String> {
        let args = PreviewArgs::parse(args)?;
        let current = self.imports.lock().unwrap_or_else(|e| e.into_inner()).epoch;
        let epoch = self.epoch.unwrap_or(current);
        if current != epoch {
            return Err("Skill preview request expired before download".into());
        }
        let packages = self.source.fetch(&args.url)?;
        if packages.is_empty() || packages.len() > MAX_CANDIDATES {
            return Err(
                "Preview between 1 and 8 skills; use a specific GitHub skill folder URL.".into(),
            );
        }
        let bytes: usize = packages
            .iter()
            .flat_map(|p| p.files.values())
            .map(Vec::len)
            .sum();
        if bytes > skills::MAX_PACKAGE_BYTES {
            return Err("Skill preview exceeds 8 MiB.".into());
        }
        let previews = packages
            .iter()
            .map(package_preview)
            .collect::<Result<Vec<_>, _>>()?;
        let message = serde_json::to_string(&json!({"previews":previews,"notice":"Review instructions before installing. New skills are available next request; installation never runs scripts."})).map_err(|_| "Could not encode skill previews.")?;
        if message.len() > MAX_PREVIEW_BYTES {
            return Err("Preview instructions exceed 64 KiB; use a smaller skill folder or import through Settings.".into());
        }
        let mut imports = self.imports.lock().unwrap_or_else(|e| e.into_inner());
        if imports.epoch != epoch {
            return Err("Skill preview request expired before publication".into());
        }
        imports.packages = previews
            .into_iter()
            .zip(packages)
            .map(|(preview, package)| (preview.token, package))
            .collect();
        Ok(ToolResult::text(message))
    }
}
impl InstallSkillTool {
    fn parse(&self, value: &Value) -> Result<InstallArgs, String> {
        let args = InstallArgs::parse(value)?;
        if !self
            .imports
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .packages
            .contains_key(&args.token)
        {
            return Err(
                "Unknown or expired skill preview token; call preview_skill again in this request."
                    .into(),
            );
        }
        Ok(args)
    }
}
impl Tool for InstallSkillTool {
    fn reset_request_state(&self) {
        let mut imports = self.imports.lock().unwrap_or_else(|e| e.into_inner());
        imports.epoch = imports.epoch.wrapping_add(1);
        imports.packages.clear();
    }
    fn name(&self) -> &str {
        "install_skill"
    }
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: self.name().into(),
            description: "Install an explicitly requested skill using a token returned by preview_skill in this request, after reviewing those instructions. Saves the exact previewed bytes without another download, overwrite, dependency installation or script execution. Personal and Project use normal runtime permissions. Returns the installed ID; its description becomes available on the next request. Do not choose a scope contrary to the user's request.".into(),
            input_schema: json!({"type":"object","required":["token","scope"],"additionalProperties":false,"properties":{"token":{"type":"string","description":"Token returned by preview_skill in this request."},"scope":{"type":"string","enum":["personal","project"]}}}),
        }
    }
    fn risk(&self, args: &Value) -> Result<ToolRisk, String> {
        Ok(match self.parse(args)?.scope {
            SkillScope::Personal => ToolRisk::ModifyOutsideWorkspace,
            SkillScope::Project => ToolRisk::ModifyWorkspace,
        })
    }
    fn summarize(&self, args: &Value) -> String {
        let Ok(args) = self.parse(args) else {
            return "Invalid skill installation".into();
        };
        let imports = self.imports.lock().unwrap_or_else(|e| e.into_inner());
        let package = &imports.packages[&args.token];
        summarize_arguments(
            &json!({"scope":args.scope,"source":package.source,"fingerprint":skills::fingerprint(&package.files)}),
        )
    }
    fn execute(&self, args: &Value) -> Result<ToolResult, String> {
        let args = self.parse(args)?;
        let mut imports = self.imports.lock().unwrap_or_else(|e| e.into_inner());
        let package = imports.packages[&args.token].clone();
        let info = imports.store.install(package, args.scope, false)?;
        imports.packages.remove(&args.token);
        Ok(ToolResult::text(
            serde_json::to_string(&info).map_err(|_| "Could not encode installed skill.")?,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        agents::{AgentRegistry, LocalAgent, LocalAgentDefinition},
        models::{ModelProvider, ModelRequest},
        permissions::{
            AuthorizationDecision, AuthorizationRequest, PermissionMode, ToolAuthorizer,
        },
        skills::SkillSource,
        Runtime,
    };
    use serde_json::{json, Value};
    use std::{
        cell::{Cell, RefCell},
        collections::VecDeque,
        sync::atomic::{AtomicUsize, Ordering},
    };

    const URL: &str = "https://github.com/team/skills";
    struct Source {
        calls: Arc<AtomicUsize>,
        packages: Vec<SkillPackage>,
    }
    impl ImportSource for Source {
        fn fetch(&self, url: &str) -> Result<Vec<SkillPackage>, String> {
            assert_eq!(url, URL);
            self.calls.fetch_add(1, Ordering::SeqCst);
            Ok(self.packages.clone())
        }
    }
    fn package() -> SkillPackage {
        SkillPackage {
            source: Some(SkillSource { repository: "team/skills".into(), reference: "main".into(), revision: "a".repeat(40), path: "review".into() }),
            files: BTreeMap::from([
                ("SKILL.md".into(), b"---\nname: review\ndescription: Review code changes\n---\nRead references/guide.txt before reviewing.".to_vec()),
                ("references/guide.txt".into(), b"Frozen supporting instructions".to_vec()),
            ]),
        }
    }
    fn fixture(
        packages: Vec<SkillPackage>,
    ) -> (
        tempfile::TempDir,
        PreviewSkillTool,
        InstallSkillTool,
        Arc<AtomicUsize>,
    ) {
        let root = tempfile::tempdir().unwrap();
        let calls = Arc::new(AtomicUsize::new(0));
        let store = SkillStore::new(root.path().join("personal"), root.path().join("workspace"));
        let (preview, install) = tools(
            store,
            Box::new(Source {
                calls: calls.clone(),
                packages,
            }),
        );
        (root, preview, install, calls)
    }
    fn preview_token(preview: &PreviewSkillTool) -> String {
        let result: Value =
            serde_json::from_str(&preview.execute(&json!({"url": URL})).unwrap().message).unwrap();
        assert!(result["previews"][0]["content"]
            .as_str()
            .unwrap()
            .contains("Read references/guide.txt"));
        assert_eq!(
            result["previews"][0]["files"],
            json!(["SKILL.md", "references/guide.txt"])
        );
        result["previews"][0]["token"].as_str().unwrap().to_string()
    }
    #[test]
    fn preview_has_no_filesystem_side_effect_and_install_uses_exact_frozen_bytes() {
        let (root, preview, install, calls) = fixture(vec![package()]);
        let token = preview_token(&preview);
        assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 0);
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        let args = json!({"token": token, "scope": "project"});
        assert_eq!(install.risk(&args).unwrap(), ToolRisk::ModifyWorkspace);
        let result: Value = serde_json::from_str(&install.execute(&args).unwrap().message).unwrap();
        assert_eq!(result["id"], "project:review");
        assert_eq!(
            std::fs::read(
                root.path()
                    .join("workspace/.agents/skills/review/references/guide.txt")
            )
            .unwrap(),
            b"Frozen supporting instructions"
        );
        assert_eq!(
            calls.load(Ordering::SeqCst),
            1,
            "installation must not refetch a mutable remote source"
        );
        assert!(install.execute(&args).is_err());
    }
    #[test]
    fn personal_install_has_outside_workspace_risk_and_no_overwrite() {
        let (root, preview, install, _) = fixture(vec![package()]);
        let token = preview_token(&preview);
        let args = json!({"token": token, "scope": "personal"});
        assert_eq!(
            install.risk(&args).unwrap(),
            ToolRisk::ModifyOutsideWorkspace
        );
        install.execute(&args).unwrap();
        let token = preview_token(&preview);
        assert!(install
            .execute(&json!({"token": token,"scope":"personal"}))
            .is_err());
        assert_eq!(
            std::fs::read(root.path().join("personal/review/SKILL.md")).unwrap(),
            package().files["SKILL.md"]
        );
    }
    #[test]
    fn rejects_unknown_stale_tokens_and_invalid_inputs_without_acquisition() {
        let (root, preview, install, calls) = fixture(vec![package()]);
        for args in [
            json!({"url":"https://evil.test/repo"}),
            json!({"url":URL,"extra":true}),
            json!({"url":"https://user:secret@github.com/team/skills"}),
        ] {
            assert!(preview.risk(&args).is_err());
            assert!(preview.execute(&args).is_err());
        }
        assert_eq!(calls.load(Ordering::SeqCst), 0);
        for args in [
            json!({"token":"forged","scope":"project"}),
            json!({"token":uuid::Uuid::new_v4().to_string(),"scope":"project"}),
            json!({"token":"x","scope":"global"}),
        ] {
            assert!(install.risk(&args).is_err());
            assert!(install.execute(&args).is_err());
        }
        let old = preview_token(&preview);
        let _new = preview_token(&preview);
        assert!(install
            .execute(&json!({"token":old,"scope":"project"}))
            .is_err());
        assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 0);
    }
    #[test]
    fn rejects_oversized_preview_batches_and_responses_without_retaining_tokens() {
        for packages in [
            vec![package(); 9],
            {
                let mut package = package();
                package.files.insert(
                    "SKILL.md".into(),
                    format!(
                        "---\nname: review\ndescription: Review\n---\n{}",
                        "x".repeat(65 * 1024)
                    )
                    .into_bytes(),
                );
                vec![package]
            },
            {
                let mut package = package();
                for i in 0..33 {
                    package
                        .files
                        .insert(format!("data/{i}.bin"), vec![0; 256 * 1024]);
                }
                vec![package]
            },
        ] {
            let (root, preview, install, _) = fixture(packages);
            assert!(preview.execute(&json!({"url": URL})).is_err());
            assert!(install
                .imports
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .packages
                .is_empty());
            assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 0);
        }
    }
    struct QueueModel(RefCell<VecDeque<String>>);
    impl ModelProvider for QueueModel {
        fn complete(&self, _: &ModelRequest) -> Result<String, String> {
            self.0
                .borrow_mut()
                .pop_front()
                .ok_or("unexpected model call".into())
        }
    }
    struct Deny;
    impl ToolAuthorizer for Deny {
        fn authorize(&self, _: &AuthorizationRequest) -> Result<AuthorizationDecision, String> {
            Ok(AuthorizationDecision::Deny)
        }
    }
    fn runtime(tool: Box<dyn Tool>, args: Value) -> Runtime {
        let definition = LocalAgentDefinition::from_toml("name = 'atlas'\ndisplay_name = 'Atlas'\nmodel_provider = 'fake'\nmodel = 'fake'\nsystem_prompt = 'Assistant'\nconversation_title_prompt = 'Title'").unwrap();
        let name = tool.name().to_string();
        let model = QueueModel(RefCell::new(VecDeque::from([
            format!(r#"{{"type":"tool_call","tool":"{name}","args":{args}}}"#),
            r#"{"type":"final","content":"Finished"}"#.to_string(),
        ])));
        Runtime::new(AgentRegistry::new(vec![Box::new(LocalAgent::new(
            definition,
            Box::new(model),
        ))]))
        .with_tools(ToolRegistry::new(vec![tool]))
    }
    #[test]
    fn denied_preview_never_downloads_or_creates_directories() {
        let (root, preview, _install, calls) = fixture(vec![package()]);
        runtime(Box::new(preview), json!({"url": URL}))
            .ask_stream_with_history_and_authorization(
                "Install this skill",
                &[],
                PermissionMode::Workspace,
                &Deny,
                &mut |_| {},
                &mut |_| {},
            )
            .unwrap();
        assert_eq!(calls.load(Ordering::SeqCst), 0);
        assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 0);
    }

    struct ImportModel {
        step: Cell<usize>,
        token: RefCell<String>,
        install_in_same_request: bool,
    }
    impl ModelProvider for ImportModel {
        fn complete(&self, request: &ModelRequest) -> Result<String, String> {
            let step = self.step.get();
            self.step.set(step + 1);
            match step {
                0 => Ok(format!(
                    r#"{{"type":"tool_call","tool":"preview_skill","args":{{"url":"{URL}"}}}}"#
                )),
                1 => {
                    let continuation: Value = serde_json::from_str(request.user_prompt).unwrap();
                    let preview: Value = serde_json::from_str(
                        continuation["tool_results"][0]["output"].as_str().unwrap(),
                    )
                    .unwrap();
                    *self.token.borrow_mut() =
                        preview["previews"][0]["token"].as_str().unwrap().into();
                    if self.install_in_same_request {
                        self.install_request()
                    } else {
                        Ok(r#"{"type":"final","content":"Preview ready"}"#.into())
                    }
                }
                2 if !self.install_in_same_request => self.install_request(),
                _ => Ok(r#"{"type":"final","content":"Finished"}"#.into()),
            }
        }
    }
    impl ImportModel {
        fn install_request(&self) -> Result<String, String> {
            Ok(format!(
                r#"{{"type":"tool_call","tool":"install_skill","args":{{"token":"{}","scope":"personal"}}}}"#,
                self.token.borrow()
            ))
        }
    }
    fn import_runtime(
        preview: PreviewSkillTool,
        install: InstallSkillTool,
        same_request: bool,
    ) -> Runtime {
        let definition = LocalAgentDefinition::from_toml("name = 'atlas'\ndisplay_name = 'Atlas'\nmodel_provider = 'fake'\nmodel = 'fake'\nsystem_prompt = 'Assistant'\nconversation_title_prompt = 'Title'").unwrap();
        Runtime::new(AgentRegistry::new(vec![Box::new(LocalAgent::new(
            definition,
            Box::new(ImportModel {
                step: Cell::new(0),
                token: RefCell::new(String::new()),
                install_in_same_request: same_request,
            }),
        ))]))
        .with_tools(ToolRegistry::new(vec![
            Box::new(preview),
            Box::new(install),
        ]))
    }
    #[test]
    fn preview_tokens_expire_between_two_asks_on_one_runtime() {
        let (root, preview, install, calls) = fixture(vec![package()]);
        let imports = preview.imports.clone();
        let runtime = import_runtime(preview, install, false);
        runtime
            .ask_stream_with_history_and_authorization(
                "Preview",
                &[],
                PermissionMode::FullAccess,
                &Deny,
                &mut |_| {},
                &mut |_| {},
            )
            .unwrap();
        assert!(
            imports
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .packages
                .is_empty(),
            "successful request must release reviewed package bytes"
        );
        let mut failed_install = false;
        runtime
            .ask_stream_with_history_and_authorization(
                "Install old token",
                &[],
                PermissionMode::FullAccess,
                &Deny,
                &mut |_| {},
                &mut |event| {
                    if let crate::permissions::ToolEvent::Finished {
                        tool,
                        success,
                        details,
                        ..
                    } = event
                    {
                        if tool == "install_skill" {
                            failed_install = !success
                                && details.as_deref().unwrap_or_default().contains("expired");
                        }
                    }
                },
            )
            .unwrap();
        assert!(
            failed_install,
            "the second request must reject the previous request's token"
        );
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 0);
    }
    struct PreviewOnly(RefCell<Vec<String>>);
    impl ToolAuthorizer for PreviewOnly {
        fn authorize(
            &self,
            request: &AuthorizationRequest,
        ) -> Result<AuthorizationDecision, String> {
            self.0.borrow_mut().push(request.tool.clone());
            Ok(if request.tool == "preview_skill" {
                AuthorizationDecision::Allow
            } else {
                AuthorizationDecision::Deny
            })
        }
    }
    #[test]
    fn valid_same_request_install_still_requires_personal_approval() {
        let (root, preview, install, calls) = fixture(vec![package()]);
        let authorizer = PreviewOnly(RefCell::new(Vec::new()));
        import_runtime(preview, install, true)
            .ask_stream_with_history_and_authorization(
                "Install Personal",
                &[],
                PermissionMode::Workspace,
                &authorizer,
                &mut |_| {},
                &mut |_| {},
            )
            .unwrap();
        assert_eq!(
            *authorizer.0.borrow(),
            vec!["preview_skill", "install_skill"]
        );
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 0);
    }

    #[test]
    fn same_request_can_install_after_preview_without_losing_token() {
        let (root, preview, install, calls) = fixture(vec![package()]);
        let imports = preview.imports.clone();
        import_runtime(preview, install, true)
            .ask_stream_with_history_and_authorization(
                "Install Personal",
                &[],
                PermissionMode::FullAccess,
                &Deny,
                &mut |_| {},
                &mut |_| {},
            )
            .unwrap();
        assert_eq!(
            std::fs::read(root.path().join("personal/review/SKILL.md")).unwrap(),
            package().files["SKILL.md"]
        );
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert!(imports
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .packages
            .is_empty());
    }
    struct FailingModel(Cell<bool>);
    impl ModelProvider for FailingModel {
        fn complete(&self, _: &ModelRequest) -> Result<String, String> {
            if self.0.replace(true) {
                return Err("interrupted".into());
            }
            Ok(format!(
                r#"{{"type":"tool_call","tool":"preview_skill","args":{{"url":"{URL}"}}}}"#
            ))
        }
    }
    #[test]
    fn failed_request_also_releases_preview_packages() {
        let (root, preview, install, calls) = fixture(vec![package()]);
        let imports = preview.imports.clone();
        let definition = LocalAgentDefinition::from_toml("name = 'atlas'\ndisplay_name = 'Atlas'\nmodel_provider = 'fake'\nmodel = 'fake'\nsystem_prompt = 'Assistant'\nconversation_title_prompt = 'Title'").unwrap();
        let runtime = Runtime::new(AgentRegistry::new(vec![Box::new(LocalAgent::new(
            definition,
            Box::new(FailingModel(Cell::new(false))),
        ))]))
        .with_tools(ToolRegistry::new(vec![
            Box::new(preview),
            Box::new(install),
        ]));
        let result = runtime.ask_stream_with_history_and_authorization(
            "Preview",
            &[],
            PermissionMode::FullAccess,
            &Deny,
            &mut |_| {},
            &mut |_| {},
        );
        assert_eq!(result.unwrap_err(), "interrupted");
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert!(imports
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .packages
            .is_empty());
        assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 0);
    }
}
