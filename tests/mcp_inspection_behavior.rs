use atlas_agentic::{
    mcp::{inspect_server_tools, McpConfig},
    permissions::{AuthorizationDecision, AuthorizationRequest, PermissionMode, ToolAuthorizer},
};
use std::{
    cell::Cell,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};
struct Authorizer {
    allow: bool,
    calls: Cell<usize>,
}
impl ToolAuthorizer for Authorizer {
    fn authorize(&self, request: &AuthorizationRequest) -> Result<AuthorizationDecision, String> {
        assert_eq!(request.tool, "mcp_list_tools");
        self.calls.set(self.calls.get() + 1);
        Ok(if self.allow {
            AuthorizationDecision::Allow
        } else {
            AuthorizationDecision::Deny
        })
    }
}
#[test]
fn denied_and_cancelled_inspection_cannot_launch_or_contact_a_server() {
    let root = tempfile::tempdir().unwrap();
    for mode in [PermissionMode::Ask, PermissionMode::Workspace] {
        let config = McpConfig::from_toml("[servers.local]\ncommand='/does/not/exist'").unwrap();
        let authorizer = Authorizer {
            allow: false,
            calls: Cell::new(0),
        };
        let error = inspect_server_tools(
            config,
            "local",
            root.path(),
            mode,
            &authorizer,
            &Arc::new(AtomicBool::new(false)),
        )
        .unwrap_err();
        assert!(error.contains("denied"));
        assert_eq!(authorizer.calls.get(), 1);
    }
    let config =
        McpConfig::from_toml("[servers.remote]\nurl='https://invalid.example/mcp'").unwrap();
    let authorizer = Authorizer {
        allow: true,
        calls: Cell::new(0),
    };
    let error = inspect_server_tools(
        config,
        "remote",
        root.path(),
        PermissionMode::FullAccess,
        &authorizer,
        &Arc::new(AtomicBool::new(true)),
    )
    .unwrap_err();
    assert!(error.contains("cancel"));
    assert_eq!(authorizer.calls.get(), 0);
    assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 0);
}
#[test]
fn cancellation_after_approval_is_checked_before_execution() {
    struct CancelOnApprove<'a>(&'a AtomicBool);
    impl ToolAuthorizer for CancelOnApprove<'_> {
        fn authorize(&self, _: &AuthorizationRequest) -> Result<AuthorizationDecision, String> {
            self.0.store(true, Ordering::SeqCst);
            Ok(AuthorizationDecision::Allow)
        }
    }
    let root = tempfile::tempdir().unwrap();
    let cancelled = Arc::new(AtomicBool::new(false));
    let config = McpConfig::from_toml("[servers.local]\ncommand='/does/not/exist'").unwrap();
    assert!(inspect_server_tools(
        config,
        "local",
        root.path(),
        PermissionMode::Ask,
        &CancelOnApprove(&cancelled),
        &cancelled
    )
    .unwrap_err()
    .contains("cancel"));
}
#[cfg(unix)]
#[test]
fn real_discovery_lists_tools_without_calling_them_and_releases_process() {
    let root = tempfile::tempdir().unwrap();
    let script = r#"import json,sys,os
open('started','w').write(str(os.getpid()))
for line in sys.stdin:
 r=json.loads(line)
 if 'id' not in r:continue
 if r['method']=='initialize':v={'protocolVersion':'2025-11-25','capabilities':{'tools':{}},'serverInfo':{'name':'fixture','version':'1'}}
 elif r['method']=='tools/list':v={'tools':[{'name':'echo','description':'<b>untrusted</b>','inputSchema':{'type':'object'}}]}
 else:open('business-called','w').write(r['method']);v={}
 print(json.dumps({'jsonrpc':'2.0','id':r['id'],'result':v}),flush=True)
"#;
    let mut config =
        McpConfig::from_toml("[servers.local]\ncommand='python3'\ntimeout_ms=1000").unwrap();
    config.servers.get_mut("local").unwrap().args = vec!["-c".into(), script.into()];
    let authorizer = Authorizer {
        allow: false,
        calls: Cell::new(0),
    };
    let tools = inspect_server_tools(
        config,
        "local",
        root.path(),
        PermissionMode::FullAccess,
        &authorizer,
        &Arc::new(AtomicBool::new(false)),
    )
    .unwrap();
    assert_eq!(tools.len(), 1);
    assert_eq!(tools[0].name, "echo");
    assert_eq!(tools[0].description, "<b>untrusted</b>");
    assert_eq!(authorizer.calls.get(), 0);
    assert!(!root.path().join("business-called").exists());
    let pid: i32 = std::fs::read_to_string(root.path().join("started"))
        .unwrap()
        .parse()
        .unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    while unsafe { libc::kill(pid, 0) } == 0 && std::time::Instant::now() < deadline {
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert_ne!(
        unsafe { libc::kill(pid, 0) },
        0,
        "inspection must drop its owned process"
    );
}

#[cfg(unix)]
#[test]
fn cancellation_stops_discovery_after_the_current_rpc() {
    use std::time::{Duration, Instant};
    for phase in ["initialize", "tools/list"] {
        let root = tempfile::tempdir().unwrap();
        let script = r#"import json,sys,os,time
phase=sys.argv[1]
pages=0
for line in sys.stdin:
 r=json.loads(line)
 if 'id' not in r:continue
 method=r['method']
 if method==phase:
  open('waiting','w').close()
  while not os.path.exists('release'):time.sleep(.005)
 if method=='initialize':v={'protocolVersion':'2025-11-25','capabilities':{'tools':{}},'serverInfo':{'name':'fixture','version':'1'}}
 else:
  pages+=1
  open('pages','w').write(str(pages))
  v={'tools':[],'nextCursor':str(pages)}
 print(json.dumps({'jsonrpc':'2.0','id':r['id'],'result':v}),flush=True)
"#;
        let mut config =
            McpConfig::from_toml("[servers.local]\ncommand='python3'\ntimeout_ms=2000").unwrap();
        config.servers.get_mut("local").unwrap().args =
            vec!["-c".into(), script.into(), phase.into()];
        let flag = Arc::new(AtomicBool::new(false));
        let cancel_flag = flag.clone();
        let path = root.path().to_path_buf();
        let cancel = std::thread::spawn(move || {
            let deadline = Instant::now() + Duration::from_secs(3);
            while !path.join("waiting").exists() && Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(5));
            }
            cancel_flag.store(true, Ordering::SeqCst);
            std::fs::write(path.join("release"), "").unwrap();
        });
        let authorizer = Authorizer {
            allow: false,
            calls: Cell::new(0),
        };
        let result = inspect_server_tools(
            config,
            "local",
            root.path(),
            PermissionMode::FullAccess,
            &authorizer,
            &flag,
        );
        cancel.join().unwrap();
        assert!(result.unwrap_err().contains("cancel"));
        let pages = std::fs::read_to_string(root.path().join("pages")).unwrap_or_default();
        assert_eq!(
            pages,
            if phase == "initialize" { "" } else { "1" },
            "no next RPC after cancellation"
        );
    }
}
