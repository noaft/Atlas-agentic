use atlas_agentic::{
    agent_output::AgentOutput,
    mcp::{register_tools, McpConfig},
    permissions::{requires_confirmation, PermissionMode},
    tools::ToolRegistry,
};
use serde_json::{json, Value};

#[test]
fn missing_config_is_optional_but_invalid_config_is_not() {
    let root = tempfile::tempdir().unwrap();
    assert!(McpConfig::load(&root.path().join("missing.toml"))
        .unwrap()
        .servers
        .is_empty());
    let path = root.path().join("invalid.toml");
    std::fs::write(&path, "secret-value = [").unwrap();
    let error = McpConfig::load(&path).err().unwrap();
    assert!(!error.contains("secret-value"));
}

#[test]
fn config_rejects_blank_commands_unknown_fields_and_invalid_limits() {
    for config in [
        "[servers.demo]\ncommand = ' '",
        "[servers.demo]\ncommand = 'server'\nurl = 'unsupported'",
        "[servers.demo]\ncommand = 'server'\ntimeout_ms = 0",
        "[servers.demo]\ncommand = 'server'\ntimeout_ms = 300001",
        "[servers.'']\ncommand = 'server'",
    ] {
        assert!(McpConfig::from_toml(config).is_err(), "{config}");
    }
}

fn execute(registry: &ToolRegistry, tool: &str, args: Value) -> Result<Value, String> {
    let result = registry.execute(&AgentOutput::ToolCall {
        tool: tool.into(),
        args,
        message: None,
    })?;
    assert!(result.attachments.is_empty());
    serde_json::from_str(&result.message).map_err(|e| e.to_string())
}

#[test]
fn registration_and_server_listing_never_launch_commands() {
    let config = McpConfig::from_toml("[servers.demo]\ncommand = '/does/not/exist'").unwrap();
    let root = tempfile::tempdir().unwrap();
    let mut registry = ToolRegistry::empty();
    register_tools(&mut registry, config, root.path());
    assert_eq!(
        execute(&registry, "mcp_list_servers", json!({})).unwrap(),
        json!({"servers":["demo"]})
    );
    for (tool, args) in [
        ("mcp_list_tools", json!({"server":"demo"})),
        (
            "mcp_call_tool",
            json!({"server":"demo", "tool":"echo", "arguments":{}}),
        ),
    ] {
        let auth = registry.authorization(tool, &args).unwrap();
        assert!(requires_confirmation(PermissionMode::Ask, auth.risk()));
        assert!(requires_confirmation(
            PermissionMode::Workspace,
            auth.risk()
        ));
        assert!(!requires_confirmation(
            PermissionMode::FullAccess,
            auth.risk()
        ));
    }
    assert!(registry
        .authorization("mcp_list_tools", &json!({"server":"missing"}))
        .is_err());
    assert!(registry
        .authorization(
            "mcp_call_tool",
            &json!({"server":"demo", "tool":"echo", "arguments":[]})
        )
        .is_err());
}

#[cfg(unix)]
mod stdio {
    use super::*;
    use std::time::{Duration, Instant};

    // A real child process exercises Atlas's wire protocol without a network or API key.
    const SERVER: &str = r#"
set -eu
scenario="$1"
printf started >> started
printf '%s' "$$" > pid
initialized=no
while IFS= read -r line; do
  id=$(printf '%s' "$line" | sed -n 's/.*"id":\([0-9]*\).*/\1/p')
  case "$line" in
    *'"method":"initialize"'*)
      case "$scenario" in
        timeout) sleep 10; exit 0;;
        malformed) printf 'not-json\n'; continue;;
        oversized) head -c 1048577 /dev/zero | tr '\000' x; printf '\n'; continue;;
        wrong_id) printf '{"jsonrpc":"2.0","id":999,"result":{}}\n'; continue;;
        remote_error) printf '{"jsonrpc":"2.0","id":%s,"error":{"code":-32000,"message":"secret-value"}}\n' "$id"; continue;;
      esac
      version=2025-11-25
      if [ "$scenario" = version ]; then version=2099-01-01; fi
      printf '{"jsonrpc":"2.0","id":%s,"result":{"protocolVersion":"%s","capabilities":{"tools":{}},"serverInfo":{"name":"fixture","version":"1"}}}\n' "$id" "$version";;
    *'"method":"notifications/initialized"'*) initialized=yes;;
    *'"method":"tools/list"'*)
      [ "$initialized" = yes ] || exit 9
      printf '{"jsonrpc":"2.0","method":"notifications/message","params":{"data":"secret-value"}}\n'
      if [ "$scenario" = pages ] && ! printf '%s' "$line" | grep -q '"cursor"'; then
        printf '{"jsonrpc":"2.0","id":%s,"result":{"tools":[{"name":"first","inputSchema":{"type":"object"}}],"nextCursor":"page2"}}\n' "$id"
      elif [ "$scenario" = repeated_cursor ]; then
        printf '{"jsonrpc":"2.0","id":%s,"result":{"tools":[],"nextCursor":"same"}}\n' "$id"
      else
        printf '{"jsonrpc":"2.0","id":%s,"result":{"tools":[{"name":"echo","description":"Echo","inputSchema":{"type":"object","properties":{"text":{"type":"string"}}}}]}}\n' "$id"
      fi;;
    *'"method":"tools/call"'*)
      printf called >> called
      case "$scenario" in
        call_timeout) sleep 10; exit 0;;
        tool_error) result='{"isError":true,"content":[{"type":"text","text":"tool failed"}]}';;
        image) result='{"content":[{"type":"image","data":"secret-image-data","mimeType":"image/png"}]}';;
        *) case "$line" in *'"text":"hello"'*) ;; *) exit 8;; esac
           result='{"content":[{"type":"text","text":"hello"}],"structuredContent":{"ok":true}}';;
      esac
      printf '{"jsonrpc":"2.0","id":%s,"result":%s}\n' "$id" "$result";;
  esac
done
"#;

    fn registry(root: &std::path::Path, scenario: &str, timeout: u64) -> ToolRegistry {
        std::fs::write(root.join("server.sh"), SERVER).unwrap();
        let config = McpConfig::from_toml(&format!(
            "[servers.demo]\ncommand = 'sh'\nargs = ['server.sh', '{scenario}']\ntimeout_ms = {timeout}"
        )).unwrap();
        let mut registry = ToolRegistry::empty();
        register_tools(&mut registry, config, root);
        registry
    }

    #[test]
    fn denied_runtime_calls_never_launch_a_server() {
        use atlas_agentic::{
            agents::{AgentRegistry, LocalAgent, LocalAgentDefinition},
            models::{ModelProvider, ModelRequest},
            permissions::{AuthorizationDecision, AuthorizationRequest, ToolAuthorizer},
            Runtime,
        };
        use std::{cell::RefCell, collections::VecDeque};

        struct Responses(RefCell<VecDeque<String>>);
        impl ModelProvider for Responses {
            fn complete(&self, _: &ModelRequest) -> Result<String, String> {
                Ok(self.0.borrow_mut().pop_front().unwrap())
            }
        }
        struct Deny;
        impl ToolAuthorizer for Deny {
            fn authorize(&self, _: &AuthorizationRequest) -> Result<AuthorizationDecision, String> {
                Ok(AuthorizationDecision::Deny)
            }
        }
        for mode in [PermissionMode::Ask, PermissionMode::Workspace] {
            for (tool, args) in [
                ("mcp_list_tools", json!({"server":"demo"})),
                (
                    "mcp_call_tool",
                    json!({"server":"demo","tool":"echo","arguments":{}}),
                ),
            ] {
                let root = tempfile::tempdir().unwrap();
                let tools = registry(root.path(), "success", 2000);
                let definition = LocalAgentDefinition::from_toml(
                    r#"
name = "atlas"
display_name = "Atlas"
model_provider = "fake"
model = "fake"
system_prompt = "Test agent"
conversation_title_prompt = "Title"
"#,
                )
                .unwrap();
                let model = Responses(RefCell::new(VecDeque::from([
                    format!(r#"{{"type":"tool_call","tool":"{tool}","args":{args}}}"#),
                    r#"{"type":"final","content":"Denied"}"#.to_string(),
                ])));
                let runtime = Runtime::new(AgentRegistry::new(vec![Box::new(LocalAgent::new(
                    definition,
                    Box::new(model),
                ))]))
                .with_tools(tools);
                let result = runtime.ask_stream_with_history_and_authorization(
                    "test",
                    &[],
                    mode,
                    &Deny,
                    &mut |_| {},
                    &mut |_| {},
                );
                assert_eq!(result.unwrap(), "Denied");
                assert!(!root.path().join("started").exists());
            }
        }
    }

    #[test]
    fn initializes_discovers_and_calls_on_one_connection() {
        let root = tempfile::tempdir().unwrap();
        let registry = registry(root.path(), "success", 2000);
        assert!(!root.path().join("started").exists());
        let tools = execute(&registry, "mcp_list_tools", json!({"server":"demo"})).unwrap();
        assert_eq!(tools["tools"][0]["name"], "echo");
        assert_eq!(tools["tools"][0]["inputSchema"]["type"], "object");
        let result = execute(
            &registry,
            "mcp_call_tool",
            json!({"server":"demo","tool":"echo","arguments":{"text":"hello"}}),
        )
        .unwrap();
        assert_eq!(result["content"][0]["text"], "hello");
        assert_eq!(result["structuredContent"]["ok"], true);
        assert_eq!(
            std::fs::read_to_string(root.path().join("started")).unwrap(),
            "started"
        );
    }

    #[test]
    fn discovery_collects_pages_and_rejects_repeated_cursors() {
        let root = tempfile::tempdir().unwrap();
        let registry = registry(root.path(), "pages", 2000);
        let result = execute(&registry, "mcp_list_tools", json!({"server":"demo"})).unwrap();
        assert_eq!(result["tools"].as_array().unwrap().len(), 2);
        let other = tempfile::tempdir().unwrap();
        let repeated = self::registry(other.path(), "repeated_cursor", 2000);
        assert!(execute(&repeated, "mcp_list_tools", json!({"server":"demo"})).is_err());
    }

    #[test]
    fn rejects_bad_protocol_without_echoing_server_data() {
        for scenario in [
            "malformed",
            "oversized",
            "wrong_id",
            "remote_error",
            "version",
        ] {
            let root = tempfile::tempdir().unwrap();
            let registry = registry(root.path(), scenario, 2000);
            let error = execute(&registry, "mcp_list_tools", json!({"server":"demo"})).unwrap_err();
            assert!(!error.contains("secret-value"), "{error}");
        }
    }

    #[test]
    fn hanging_server_times_out_and_can_be_reconnected_without_replay() {
        let root = tempfile::tempdir().unwrap();
        let registry = registry(root.path(), "timeout", 100);
        let start = Instant::now();
        assert!(
            execute(&registry, "mcp_list_tools", json!({"server":"demo"}))
                .unwrap_err()
                .contains("timed out")
        );
        assert!(start.elapsed() < Duration::from_secs(2));
        assert!(execute(&registry, "mcp_list_tools", json!({"server":"demo"})).is_err());
        assert_eq!(
            std::fs::read_to_string(root.path().join("started")).unwrap(),
            "startedstarted"
        );
    }

    #[test]
    fn timed_out_tool_calls_are_not_replayed_and_owned_process_is_reaped() {
        let root = tempfile::tempdir().unwrap();
        let registry = registry(root.path(), "call_timeout", 500);
        let error = execute(
            &registry,
            "mcp_call_tool",
            json!({"server":"demo","tool":"echo","arguments":{"text":"hello"}}),
        )
        .unwrap_err();
        assert!(error.contains("timed out"));
        assert_eq!(
            std::fs::read_to_string(root.path().join("called")).unwrap(),
            "called"
        );
        let pid: i32 = std::fs::read_to_string(root.path().join("pid"))
            .unwrap()
            .parse()
            .unwrap();
        assert_eq!(unsafe { libc::kill(pid, 0) }, -1);
    }

    #[test]
    fn dropping_registry_terminates_its_server() {
        let root = tempfile::tempdir().unwrap();
        let registry = registry(root.path(), "success", 2000);
        execute(&registry, "mcp_list_tools", json!({"server":"demo"})).unwrap();
        let pid: i32 = std::fs::read_to_string(root.path().join("pid"))
            .unwrap()
            .parse()
            .unwrap();
        assert_eq!(unsafe { libc::kill(pid, 0) }, 0);
        drop(registry);
        assert_eq!(unsafe { libc::kill(pid, 0) }, -1);
    }

    #[test]
    fn tool_errors_and_binary_results_never_become_successful_text() {
        for scenario in ["tool_error", "image"] {
            let root = tempfile::tempdir().unwrap();
            let registry = registry(root.path(), scenario, 2000);
            let error = execute(
                &registry,
                "mcp_call_tool",
                json!({"server":"demo","tool":"echo","arguments":{"text":"hello"}}),
            )
            .unwrap_err();
            assert!(!error.contains("secret-image-data"));
        }
    }

    #[test]
    fn tool_errors_preserve_corrective_details_and_keep_the_session() {
        let root = tempfile::tempdir().unwrap();
        let registry = registry(root.path(), "tool_error", 2000);
        let error = execute(
            &registry,
            "mcp_call_tool",
            json!({"server":"demo","tool":"echo","arguments":{}}),
        )
        .unwrap_err();
        assert!(error.contains("tool failed"), "{error}");
        execute(&registry, "mcp_list_tools", json!({"server":"demo"})).unwrap();
        assert_eq!(
            std::fs::read_to_string(root.path().join("started")).unwrap(),
            "started"
        );
    }
}

#[cfg(windows)]
#[test]
fn timeout_terminates_windows_server_descendants() {
    use atlas_agentic::mcp::ServerConfig;
    use std::{
        collections::BTreeMap,
        time::{Duration, Instant},
    };
    use windows_sys::Win32::{
        Foundation::CloseHandle,
        System::Threading::{GetExitCodeProcess, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION},
    };
    let root = tempfile::tempdir().unwrap();
    std::fs::write(root.path().join("server.ps1"), r#"
$child = Start-Process powershell.exe -ArgumentList '-NoProfile', '-NonInteractive', '-Command', 'Start-Sleep -Seconds 60' -PassThru
[IO.File]::WriteAllText((Join-Path $PWD 'child.pid'), [string]$child.Id)
Start-Sleep -Seconds 60
"#).unwrap();
    let config = McpConfig {
        servers: BTreeMap::from([(
            "demo".into(),
            ServerConfig {
                command: "powershell.exe".into(),
                args: vec![
                    "-NoProfile".into(),
                    "-NonInteractive".into(),
                    "-File".into(),
                    "server.ps1".into(),
                ],
                timeout_ms: 5000,
            },
        )]),
    };
    let mut registry = ToolRegistry::empty();
    register_tools(&mut registry, config, root.path());
    assert!(
        execute(&registry, "mcp_list_tools", json!({"server":"demo"}))
            .unwrap_err()
            .contains("timed out")
    );
    let pid: u32 = std::fs::read_to_string(root.path().join("child.pid"))
        .unwrap()
        .parse()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        let handle = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
        if handle.is_null() {
            break;
        }
        let mut code = 0;
        let read = unsafe { GetExitCodeProcess(handle, &mut code) };
        unsafe {
            CloseHandle(handle);
        }
        if read != 0 && code != 259 {
            break;
        }
        if Instant::now() >= deadline {
            let _ = std::process::Command::new("taskkill.exe")
                .args(["/PID", &pid.to_string(), "/F"])
                .output();
            panic!("MCP descendant survived timeout");
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}
