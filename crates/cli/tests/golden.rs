//! Runs `vmm --json` against a fake game and compares each result with the
//! golden file in `tests/golden/` (ADR 0003, section 8). Paths, ids, hashes
//! and times are replaced by placeholders first. To accept a new output on
//! purpose, run with `VMM_UPDATE_GOLDEN=1` and review the diff.

use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::Value;

/// The fake game: SMF with mods A (on), B and C (off); C has an option and
/// B must load after A.
fn fake_game(root: &Path) {
    let smf = root.join("Simple Mod Framework");
    std::fs::create_dir_all(root.join("Retail")).unwrap();
    std::fs::write(root.join("Retail/HITMAN3.exe"), b"").unwrap();
    std::fs::create_dir_all(smf.join("Mods")).unwrap();
    std::fs::write(smf.join("Deploy.exe"), b"").unwrap();
    std::fs::write(smf.join("config.json"), "{ loadOrder: ['A'] }").unwrap();
    for (id, extra) in [
        ("A", ", version: '1.0.0', authors: ['Someone']"),
        ("B", ", loadAfter: ['A']"),
        ("C", ", options: [{ name: 'X', type: 'checkbox', enabledByDefault: true }]"),
    ] {
        let dir = smf.join("Mods").join(id);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("manifest.json"), format!("{{ id: '{id}', name: '{id} mod'{extra} }}")).unwrap();
    }
}

struct Run {
    code: i32,
    json: Value,
}

struct Fixture {
    tmp: tempfile::TempDir,
    /// Plan ids seen so far, replaced wherever they appear.
    ids: std::cell::RefCell<Vec<String>>,
}

impl Fixture {
    fn new() -> Fixture {
        let tmp = tempfile::tempdir().unwrap();
        fake_game(&tmp.path().join("game"));
        Fixture { tmp, ids: Default::default() }
    }

    fn vmm(&self, args: &[&str]) -> Run {
        let out = Command::new(env!("CARGO_BIN_EXE_vmm"))
            .args(args)
            .arg("--json")
            .env("VAEXIL_MOD_MANAGER_GAME", self.tmp.path().join("game"))
            .env("VAEXIL_MOD_MANAGER_DATA", self.tmp.path().join("data"))
            .output()
            .unwrap();
        let text = String::from_utf8(out.stdout).unwrap();
        let json: Value = serde_json::from_str(&text).unwrap_or_else(|e| panic!("not one JSON object ({e}): {text}"));
        Run { code: out.status.code().unwrap(), json }
    }

    /// Replaces what differs between runs with placeholders.
    fn normalize(&self, v: &mut Value) {
        let root = self.tmp.path().display().to_string();
        match v {
            Value::String(s) => {
                *s = s.replace(&root, "<tmp>").replace('\\', "/");
                for id in self.ids.borrow().iter() {
                    *s = s.replace(id.as_str(), "<planId>");
                }
            }
            Value::Array(a) => a.iter_mut().for_each(|x| self.normalize(x)),
            Value::Object(o) => {
                for (k, x) in o.iter_mut() {
                    match k.as_str() {
                        "planId" | "plan" | "createdAtMs" | "expiresAtMs" | "sha256" | "busy" | "appVersion"
                        | "tsMs" => {
                            if !x.is_null() {
                                *x = Value::String(format!("<{k}>"));
                            }
                        }
                        _ => self.normalize(x),
                    }
                }
            }
            _ => {}
        }
    }

    fn check(&self, name: &str, args: &[&str], code: i32) -> Value {
        let run = self.vmm(args);
        assert_eq!(run.code, code, "exit code of vmm {args:?}: {}", run.json);
        let raw = run.json.clone();
        if let Some(id) = raw["planId"].as_str() {
            self.ids.borrow_mut().push(id.to_string());
        }
        let mut got = run.json;
        self.normalize(&mut got);
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/golden").join(format!("{name}.json"));
        let text = serde_json::to_string_pretty(&got).unwrap() + "\n";
        if std::env::var_os("VMM_UPDATE_GOLDEN").is_some() {
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, &text).unwrap();
        } else {
            let want = std::fs::read_to_string(&path).unwrap_or_default().replace("\r\n", "\n");
            assert_eq!(text, want, "{name}: vmm {args:?} differs from {}", path.display());
        }
        raw
    }
}

#[test]
fn read_commands_match_their_golden_files() {
    let f = Fixture::new();
    f.check("status", &["status"], 0);
    f.check("games", &["games"], 0);
    f.check("mods", &["mods"], 0);
    f.check("mod-c", &["mod", "C"], 0);
    f.check("order", &["order"], 0);
    f.check("mod-missing", &["mod", "Z"], 3);
    f.check("unknown-command", &["frobnicate"], 2);
}

#[test]
fn a_change_is_planned_confirmed_and_applied_once() {
    let f = Fixture::new();
    let plan = f.check("plan-enable-b-c", &["plan", "enable", "B", "C"], 0);
    let id = plan["planId"].as_str().unwrap().to_string();
    f.check("apply-without-yes", &["apply", &id], 2);
    f.check("apply-yes-false", &["apply", &id, "--yes=false"], 2);
    f.check("apply", &["apply", &id, "--yes"], 0);
    f.check("apply-again", &["apply", &id, "--yes"], 3);
    f.check("order-after-apply", &["order"], 0);
    // Every change and failure so far, from the command line.
    f.check("logs-after-apply", &["logs"], 0);
    f.check("plan-bad-order", &["plan", "order", "C", "A"], 3);
    f.check("plan-bad-kind", &["plan", "kind", "A", "loud"], 2);
}

/// Plans one change and applies it, returning the plan's id.
fn plan_and_apply(f: &Fixture, name: &str, words: &[&str]) {
    let mut args = vec!["plan"];
    args.extend_from_slice(words);
    let plan = f.check(&format!("plan-{name}"), &args, 0);
    let id = plan["planId"].as_str().unwrap().to_string();
    f.check(&format!("apply-{name}"), &["apply", &id, "--yes"], 0);
}

#[test]
fn setups_and_a_compatibility_pass_from_the_command_line() {
    let f = Fixture::new();
    plan_and_apply(&f, "save-setup", &["save-setup", "Just", "A"]);
    plan_and_apply(&f, "start-pass", &["start-pass"]);
    plan_and_apply(&f, "try-batch", &["try-batch", "B", "C", "--name", "First"]);
    f.check("setups-during-pass", &["setups"], 0);
    plan_and_apply(&f, "mark-batch", &["mark-batch", "good", "works", "fine"]);
    plan_and_apply(&f, "end-pass", &["end-pass"]);
    plan_and_apply(&f, "use-setup", &["use-setup", "Just", "A"]);
    f.check("order-after-use-setup", &["order"], 0);
    f.check("deploy-without-yes", &["deploy"], 2);
}

#[test]
fn a_plan_from_a_file_and_a_stale_plan() {
    let f = Fixture::new();
    let changes = f.tmp.path().join("changes.json");
    std::fs::write(
        &changes,
        r#"{ "changes": [ { "op": "setKind", "mod": "A", "kind": "sound" }, { "op": "setEnabled", "mod": "B", "enabled": true } ] }"#,
    )
    .unwrap();
    let plan = f.check("plan-file", &["plan", "--file", changes.to_str().unwrap()], 0);
    let id = plan["planId"].as_str().unwrap().to_string();
    // Another program changes SMF's settings before the plan is applied.
    let config = f.tmp.path().join("game/Simple Mod Framework/config.json");
    std::fs::write(&config, "{ loadOrder: [] }").unwrap();
    f.check("apply-stale", &["apply", &id, "--yes"], 4);
}
