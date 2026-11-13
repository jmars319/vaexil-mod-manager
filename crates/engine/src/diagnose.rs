//! The last deploy's report and `diagnose` (ADR 0003, section 4): everything
//! needed to understand a problem, in one object, for the window's "Copy
//! diagnostics", `vmm diagnose --json` and the MCP `diagnose` tool alike.

use std::path::Path;

use vmm_core::backend::Status;
use vmm_core::game::GameInstall;
use vmm_core::schema::{DeployReport, DeployResult, Diagnosis, Problem, SCHEMA_VERSION};

use crate::{Engine, crash, deploy, enabled_order, host};

/// How many events a diagnosis includes.
pub const RECENT_EVENTS: usize = 200;

/// `text` with the user's profile folder written as `~`, so a diagnosis can
/// be shared without the account name.
pub fn tilde(text: &str, profile: &str) -> String {
    let (t, p) = (text.as_bytes(), profile.as_bytes());
    if p.len() < 4 {
        return text.to_string();
    }
    // Windows paths are case-insensitive and take either separator; ASCII
    // case is ignored and `/` matches `\`, byte by byte, so positions in
    // `text` stay exact.
    let same = |a: u8, b: u8| {
        let sep = |c: u8| c == b'/' || c == b'\\';
        a.eq_ignore_ascii_case(&b) || (sep(a) && sep(b))
    };
    let mut out = String::with_capacity(text.len());
    let (mut i, mut start) = (0, 0);
    while i + p.len() <= t.len() {
        if t[i..i + p.len()].iter().zip(p).all(|(&a, &b)| same(a, b))
            && text.is_char_boundary(i)
            && text.is_char_boundary(i + p.len())
        {
            out.push_str(&text[start..i]);
            out.push('~');
            i += p.len();
            start = i;
        } else {
            i += 1;
        }
    }
    out.push_str(&text[start..]);
    out
}

fn tilde_all(value: &mut serde_json::Value, profile: &str) {
    match value {
        serde_json::Value::String(s) => *s = tilde(s, profile),
        serde_json::Value::Array(a) => a.iter_mut().for_each(|v| tilde_all(v, profile)),
        serde_json::Value::Object(o) => o.values_mut().for_each(|v| tilde_all(v, profile)),
        _ => {}
    }
}

impl Engine {
    /// The last deploy, read again from its log, or `None` when there has
    /// been none. Works for deploys made before reports existed.
    pub fn deploy_report(&self) -> Option<DeployReport> {
        let record = self.record();
        record.at_ms?;
        let text = record.log.as_deref().and_then(|p| std::fs::read_to_string(p).ok()).unwrap_or_default();
        let parts = self.backend().read_deploy_log(&text);
        let result = match (record.success, record.exit_code) {
            (Some(true), _) => DeployResult::Succeeded,
            (_, Some(_)) => DeployResult::Failed,
            _ => DeployResult::DidNotRun,
        };
        Some(DeployReport {
            schema_version: SCHEMA_VERSION,
            result,
            exit_code: record.exit_code,
            started_at_ms: record.started_at_ms,
            finished_at_ms: record.at_ms,
            duration_ms: record.started_at_ms.zip(record.at_ms).map(|(s, e)| e.saturating_sub(s)),
            stages: parts.stages,
            warnings: parts.warnings,
            errors: parts.errors,
            log: record.log.clone(),
            log_lines: text.lines().count(),
        })
    }

    /// Everything about this install and the app at once. Never fails: what
    /// can't be read is reported as a problem.
    pub fn diagnose(&self, game: Option<&GameInstall>) -> Diagnosis {
        let mut detection = Vec::new();
        match host::game_override() {
            Some(g) => detection.push(format!("VAEXIL_MOD_MANAGER_GAME names {}", g.root.display())),
            None => {
                let sources = host::sources();
                detection.push(match &sources.steam {
                    Some(p) => format!("Steam found at {}", p.display()),
                    None => "Steam not found".into(),
                });
                detection.push(match &sources.epic_manifests {
                    Some(p) => format!("Epic Games manifests at {}", p.display()),
                    None => "Epic Games manifests not found".into(),
                });
                detection.push(format!("{} XboxGames folder(s)", sources.xbox_roots.len()));
            }
        }
        for g in self.installs() {
            detection.push(format!("Install: {} ({})", g.root.display(), g.store.label()));
        }
        if let Some(g) = game {
            detection.push(format!("Using: {}", g.root.display()));
        }

        let mut problems = Vec::new();
        let problem = |code: &str, mod_id: Option<&str>, detail: String| Problem {
            code: code.into(),
            mod_id: mod_id.map(str::to_string),
            detail,
        };
        if let Some(why) = self.settings_problem() {
            problems.push(problem("settings.unreadable", None, why));
        }
        if let Err(e) = self.setups() {
            problems.push(problem("settings.unreadable", None, e.message));
        }
        if let Some(g) = game {
            let inspection = self.inspect(g);
            match &inspection.status {
                Status::NotInstalled => {
                    problems.push(problem("framework.missing", None, "SMF isn't in the game folder.".into()))
                }
                Status::Unreadable { reason, .. } => problems.push(problem("config.unreadable", None, reason.clone())),
                Status::Installed { .. } => {}
            }
            for m in &inspection.mods {
                if let Some(p) = &m.problem {
                    problems.push(problem("mod.unreadable", Some(&m.id), p.clone()));
                }
            }
            let order = enabled_order(&inspection.mods);
            for b in vmm_core::order::broken_rules(&order, &inspection.mods) {
                let rel = if b.relation == vmm_core::backend::Relation::Before { "before" } else { "after" };
                problems.push(problem(
                    "order.rule_broken",
                    Some(&b.mod_id),
                    format!("{} should load {rel} {}.", b.mod_id, b.other),
                ));
            }
            for n in &inspection.notes {
                problems.push(problem("backend.note", None, n.clone()));
            }
        } else {
            problems.push(problem("game.not_found", None, "No install of the game was found.".into()));
        }

        let settings = self.data_dir().and_then(|d| std::fs::read_to_string(d.join("settings.json")).ok());
        let settings = settings
            .and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok())
            .map(|v| {
                serde_json::json!({
                    "aiChanges": v.get("aiChanges").and_then(|x| x.as_bool()).unwrap_or(false),
                    "typesChosen": v.get("kindOverrides").and_then(|x| x.as_object()).map_or(0, |o| o.len()),
                })
            })
            .unwrap_or_else(|| serde_json::json!({ "aiChanges": false, "typesChosen": 0 }));
        let settings = match self.data_dir() {
            Some(d) => {
                let mut s = settings;
                s["dataFolder"] = serde_json::Value::String(d.display().to_string());
                s
            }
            None => settings,
        };
        let crashes = self
            .data_dir()
            .map(|d| crash::records(d).into_iter().take(5).map(|p| p.display().to_string()).collect())
            .unwrap_or_default();

        let diagnosis = Diagnosis {
            schema_version: SCHEMA_VERSION,
            app_version: env!("CARGO_PKG_VERSION").into(),
            generated_at_ms: deploy::now_ms(),
            detection,
            status: self.status(game),
            problems,
            last_deploy: self.deploy_report(),
            recent_events: self.recent_events(RECENT_EVENTS),
            settings,
            crashes,
        };
        match std::env::var("USERPROFILE").ok().filter(|p| Path::new(p).is_absolute()) {
            Some(profile) => {
                let mut v = serde_json::to_value(&diagnosis).expect("a diagnosis serializes");
                tilde_all(&mut v, &profile);
                serde_json::from_value(v).unwrap_or(diagnosis)
            }
            None => diagnosis,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_profile_folder_becomes_a_tilde_whatever_its_case() {
        let p = r"C:\Users\Jason";
        assert_eq!(tilde(r"C:\users\jason\AppData\x.log", p), r"~\AppData\x.log");
        assert_eq!(tilde(r"a C:\Users\Jason b C:\Users\Jason", p), "a ~ b ~");
        assert_eq!(tilde(r"D:\Games\HITMAN 3", p), r"D:\Games\HITMAN 3");
        // Either separator, as a path set by hand may use.
        assert_eq!(tilde("C:/Users/Jason/vmm-data", p), "~/vmm-data");
        assert_eq!(tilde(r"c:\users/jason\x", p), r"~\x");
    }

    #[test]
    fn a_deploy_report_is_read_from_the_saved_log() {
        let tmp = tempfile::tempdir().unwrap();
        let data = tmp.path().join("data");
        let log = tmp.path().join("deploy.log");
        std::fs::write(
            &log,
            "INFO\tDiscovering mod contents\nWARN\tMod M replaces a base game entity file (00AA) with a raw file.\nINFO\tDone in 1s\n",
        )
        .unwrap();
        let record = deploy::Record {
            fingerprint: None,
            success: Some(true),
            exit_code: Some(0),
            log: Some(log.display().to_string()),
            at_ms: Some(5_000),
            started_at_ms: Some(1_000),
        };
        record.save(&data).unwrap();
        let report = Engine::with_data_dir(Some(data)).deploy_report().unwrap();
        assert_eq!(report.result, DeployResult::Succeeded);
        assert_eq!(report.duration_ms, Some(4_000));
        assert_eq!(report.stages.len(), 2);
        assert_eq!(report.warnings[0].count, 1);
        assert_eq!(report.log_lines, 3);
        assert!(Engine::with_data_dir(Some(tmp.path().join("none"))).deploy_report().is_none());
    }
}
