//! Starts `vmm mcp` against a fake game and speaks MCP to it over stdio, as
//! an AI client would (ADR 0003, section 8): the handshake, the tool list
//! with its annotations and schemas, every read tool, and the rule that a
//! change from an AI tool is refused until Settings allows it.

use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};

use serde_json::{Value, json};

fn fake_game(root: &Path) {
    let smf = root.join("Simple Mod Framework");
    std::fs::create_dir_all(root.join("Retail")).unwrap();
    std::fs::write(root.join("Retail/HITMAN3.exe"), b"").unwrap();
    std::fs::create_dir_all(smf.join("Mods")).unwrap();
    std::fs::write(smf.join("Deploy.exe"), b"").unwrap();
    std::fs::write(smf.join("config.json"), "{ loadOrder: ['A'] }").unwrap();
    for id in ["A", "B"] {
        let dir = smf.join("Mods").join(id);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("manifest.json"), format!("{{ id: '{id}', name: '{id} mod' }}")).unwrap();
    }
}

struct Server {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    next: u64,
}

impl Server {
    fn start(tmp: &Path) -> Server {
        let mut child = Command::new(env!("CARGO_BIN_EXE_vmm"))
            .arg("mcp")
            .env("VAEXIL_MOD_MANAGER_GAME", tmp.join("game"))
            .env("VAEXIL_MOD_MANAGER_DATA", tmp.join("data"))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .unwrap();
        let stdin = child.stdin.take().unwrap();
        let stdout = BufReader::new(child.stdout.take().unwrap());
        Server { child, stdin, stdout, next: 1 }
    }

    fn send(&mut self, message: Value) {
        writeln!(self.stdin, "{message}").unwrap();
        self.stdin.flush().unwrap();
    }

    /// Sends a request and returns its result, skipping any notifications.
    fn request(&mut self, method: &str, params: Value) -> Value {
        let id = self.next;
        self.next += 1;
        self.send(json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params }));
        loop {
            let mut line = String::new();
            assert!(self.stdout.read_line(&mut line).unwrap() > 0, "the server closed stdout");
            let msg: Value = serde_json::from_str(&line).unwrap_or_else(|e| panic!("not JSON-RPC ({e}): {line}"));
            if msg["id"] == json!(id) {
                assert!(msg.get("error").is_none(), "{method} failed: {msg}");
                return msg["result"].clone();
            }
        }
    }

    fn call(&mut self, tool: &str, args: Value) -> Value {
        self.request("tools/call", json!({ "name": tool, "arguments": args }))
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn error_code(result: &Value) -> String {
    assert_eq!(result["isError"], json!(true), "{result}");
    let text = result["content"][0]["text"].as_str().unwrap();
    serde_json::from_str::<Value>(text).unwrap()["error"]["code"].as_str().unwrap().to_string()
}

#[test]
fn an_ai_client_can_read_everything_and_change_only_when_allowed() {
    let tmp = tempfile::tempdir().unwrap();
    fake_game(&tmp.path().join("game"));
    let mut s = Server::start(tmp.path());

    let init = s.request(
        "initialize",
        json!({
            "protocolVersion": "2025-06-18",
            "capabilities": {},
            "clientInfo": { "name": "test-client", "version": "1.0" }
        }),
    );
    assert_eq!(init["serverInfo"]["name"], "vaexil-mod-manager");
    assert!(init["instructions"].as_str().unwrap().contains("Change nothing on your own initiative"));
    s.send(json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }));

    // Every tool is listed with schemas; only apply_plan may change anything.
    let tools = s.request("tools/list", json!({}))["tools"].as_array().unwrap().clone();
    let mut names: Vec<&str> = tools.iter().map(|t| t["name"].as_str().unwrap()).collect();
    names.sort();
    assert_eq!(
        names,
        [
            "apply_plan",
            "diagnose",
            "get_deploy_report",
            "get_deploy_status",
            "get_load_order",
            "get_logs",
            "get_mod",
            "get_setups",
            "get_smf_update",
            "get_status",
            "list_error_codes",
            "list_games",
            "list_mods",
            "plan_change",
            "start_deploy"
        ]
    );
    for t in &tools {
        assert!(t["outputSchema"].is_object(), "{} has no output schema", t["name"]);
        let a = &t["annotations"];
        let writes = t["name"] == "apply_plan" || t["name"] == "start_deploy";
        assert_eq!(a["readOnlyHint"], json!(!writes), "{}", t["name"]);
        assert_eq!(a["destructiveHint"], json!(writes), "{}", t["name"]);
        assert_eq!(a["openWorldHint"], json!(false), "{}", t["name"]);
    }

    // The read tools.
    assert_eq!(s.call("list_games", json!({}))["structuredContent"]["games"].as_array().unwrap().len(), 1);
    let status = s.call("get_status", json!({}));
    assert_eq!(status["structuredContent"]["enabled"], 1);
    assert_eq!(status["structuredContent"]["backend"]["nativeMods"]["status"], "none");
    assert_eq!(s.call("list_mods", json!({}))["structuredContent"]["mods"].as_array().unwrap().len(), 2);
    assert_eq!(s.call("get_mod", json!({ "mod": "B" }))["structuredContent"]["mod"]["enabled"], false);
    assert_eq!(error_code(&s.call("get_mod", json!({ "mod": "Z" }))), "mod.not_found");
    assert_eq!(s.call("get_load_order", json!({}))["structuredContent"]["order"], json!(["A"]));
    assert!(!s.call("list_error_codes", json!({}))["structuredContent"]["codes"].as_array().unwrap().is_empty());

    // A plan changes nothing; applying it is refused while Settings says no.
    let config = tmp.path().join("game/Simple Mod Framework/config.json");
    let before = std::fs::read(&config).unwrap();
    let plan = s.call("plan_change", json!({ "changes": [{ "op": "setEnabled", "mod": "B", "enabled": true }] }));
    let plan_id = plan["structuredContent"]["planId"].as_str().unwrap().to_string();
    let apply = json!({ "planId": plan_id, "request": "turn on B" });
    assert_eq!(error_code(&s.call("apply_plan", apply.clone())), "ai.changes_off");
    assert_eq!(std::fs::read(&config).unwrap(), before);

    // A deploy is a change too: refused while Settings says no.
    let deploy = s.call("start_deploy", json!({ "request": "deploy" }));
    assert_eq!(error_code(&deploy), "ai.changes_off");
    assert_eq!(s.call("get_setups", json!({}))["structuredContent"]["pass"], Value::Null);

    // Allowed in Settings (what the window's switch writes), it applies once
    // and the journal records the client and the request.
    std::fs::write(tmp.path().join("data/settings.json"), r#"{ "aiChanges": true }"#).unwrap();
    assert_eq!(error_code(&s.call("apply_plan", json!({ "planId": plan_id, "request": " " }))), "usage.invalid");
    let applied = s.call("apply_plan", apply);
    assert_eq!(applied["structuredContent"]["steps"][0]["changesSomething"], true, "{applied}");
    assert_ne!(std::fs::read(&config).unwrap(), before);
    let journal = std::fs::read_to_string(tmp.path().join("data/journal.jsonl")).unwrap();
    let entry: Value = serde_json::from_str(journal.lines().last().unwrap()).unwrap();
    assert_eq!((entry["caller"].as_str(), entry["request"].as_str()), (Some("mcp:test-client"), Some("turn on B")));

    // The event log has the refusal and the change, each with the client.
    let events = s.call("get_logs", json!({ "limit": 20 }))["structuredContent"]["events"].as_array().unwrap().clone();
    let seen: Vec<(String, String, String)> = events
        .iter()
        .map(|e| {
            let f = |k: &str| e[k].as_str().unwrap_or_default().to_string();
            (f("event"), f("level"), f("caller"))
        })
        .collect();
    let has =
        |event: &str, level: &str| seen.iter().any(|(e, l, c)| e == event && l == level && c == "mcp:test-client");
    assert!(has("plan.made", "info"), "{seen:?}");
    assert!(has("plan.applied", "error"), "the refusal is logged: {seen:?}");
    assert!(has("mod.enabled", "info"), "{seen:?}");
    assert!(has("plan.applied", "info"), "{seen:?}");

    // No deploy yet; a diagnosis still comes back whole.
    assert_eq!(s.call("get_deploy_report", json!({}))["structuredContent"]["report"], Value::Null);
    let d = s.call("diagnose", json!({}))["structuredContent"].clone();
    assert_eq!(d["status"]["enabled"], 2);
    assert!(d["recentEvents"].as_array().unwrap().len() >= 4);
}
