//! Deploy and Play: the framework's deploy runs on a worker thread, its
//! output goes to a log in the app's folder, and front ends poll its progress.
//! Neither runs while HITMAN, or another deploy, is running. Play deploys
//! first when anything changed since the last deploy, then starts HITMAN
//! through Steam.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{Map, Value};
use vmm_backend_smf::Smf;
use vmm_core::backend::{Backend, DeployOutcome};
use vmm_core::error::{ErrorCode, VmmError};
use vmm_core::game::{GameInstall, STEAM_APP_ID, Store};

use crate::store;

const RECORD: &str = "deploy-state.json";
/// How many deploy logs the app keeps.
pub const LOGS_KEPT: usize = 20;
/// The processes that block a deploy: the game, and SMF's own deploy.
pub const BLOCKERS: [&str; 2] = ["HITMAN3.exe", "Deploy.exe"];

/// The last finished deploy, as remembered in `deploy-state.json`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Record {
    /// The fingerprint deployed, when the deploy succeeded.
    pub fingerprint: Option<String>,
    pub success: Option<bool>,
    pub exit_code: Option<i32>,
    pub log: Option<String>,
    pub at_ms: Option<u64>,
}

impl Record {
    pub fn load(dir: &Path) -> Record {
        let Ok(m) = store::load(dir, RECORD) else { return Record::default() };
        Record {
            fingerprint: m.get("fingerprint").and_then(Value::as_str).map(str::to_string),
            success: m.get("success").and_then(Value::as_bool),
            exit_code: m.get("exitCode").and_then(Value::as_i64).map(|c| c as i32),
            log: m.get("log").and_then(Value::as_str).map(str::to_string),
            at_ms: m.get("at").and_then(Value::as_u64),
        }
    }

    pub fn save(&self, dir: &Path) -> Result<(), VmmError> {
        let mut m = Map::new();
        if let Some(f) = &self.fingerprint {
            m.insert("fingerprint".into(), Value::String(f.clone()));
        }
        if let Some(s) = self.success {
            m.insert("success".into(), Value::Bool(s));
        }
        if let Some(c) = self.exit_code {
            m.insert("exitCode".into(), Value::from(c));
        }
        if let Some(l) = &self.log {
            m.insert("log".into(), Value::String(l.clone()));
        }
        if let Some(t) = self.at_ms {
            m.insert("at".into(), Value::from(t));
        }
        store::save(dir, RECORD, &m)
    }
}

/// A deploy in progress, shared between the worker thread and the window.
#[derive(Debug, Default)]
pub struct Progress {
    pub running: bool,
    pub last_line: String,
    pub lines: usize,
    /// Set when the deploy has finished, until the window has taken it.
    pub finished: Option<Finished>,
}

#[derive(Debug, Clone)]
pub struct Finished {
    pub result: Result<DeployOutcome, VmmError>,
    pub fingerprint: Option<String>,
    pub log: Option<PathBuf>,
    pub then_play: bool,
}

pub type Shared = Arc<Mutex<Progress>>;

pub fn now_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_millis() as u64)
}

/// Why a deploy or Play can't start now, if it can't.
pub fn blocked(running: &[String]) -> Option<VmmError> {
    if running.iter().any(|r| r.eq_ignore_ascii_case("HITMAN3.exe")) {
        Some(VmmError::new(ErrorCode::GameRunning, "HITMAN is running. Close it to deploy."))
    } else if running.iter().any(|r| r.eq_ignore_ascii_case("Deploy.exe")) {
        Some(VmmError::new(ErrorCode::DeployRunning, "SMF's Deploy.exe is already running. Wait for it to finish."))
    } else {
        None
    }
}

/// Starts a deploy on a worker thread. `logs` is where its log goes.
pub fn start(progress: &Shared, game: GameInstall, logs: Option<PathBuf>, then_play: bool) {
    {
        let mut p = progress.lock().expect("deploy progress lock");
        if p.running {
            return;
        }
        *p = Progress { running: true, last_line: "Starting SMF's Deploy.exe…".into(), lines: 0, finished: None };
    }
    let progress = progress.clone();
    std::thread::spawn(move || {
        let smf = Smf::default();
        // The fingerprint before the deploy: a change made while it runs
        // then shows as "changes since the last deploy".
        let fingerprint = smf.deploy_fingerprint(&game).ok();
        let log_path = logs.as_ref().and_then(|dir| new_log(dir).ok());
        let mut log = log_path.as_ref().and_then(|p| std::fs::File::create(p).ok());
        let result = smf.deploy(&game, &mut |line| {
            if let Some(f) = log.as_mut() {
                let _ = writeln!(f, "{line}");
            }
            if let Ok(mut p) = progress.lock()
                && !line.trim().is_empty()
            {
                p.last_line = line.trim().to_string();
                p.lines += 1;
            }
        });
        if let (Some(f), Err(e)) = (log.as_mut(), &result) {
            let _ = writeln!(f, "Vaexil Mod Manager: {e}");
        }
        drop(log);
        if let Ok(mut p) = progress.lock() {
            p.running = false;
            p.finished = Some(Finished { result, fingerprint, log: log_path, then_play });
        }
    });
}

/// A new log file name in `dir`, keeping the newest [`LOGS_KEPT`].
fn new_log(dir: &Path) -> std::io::Result<PathBuf> {
    std::fs::create_dir_all(dir)?;
    let mut old: Vec<PathBuf> = std::fs::read_dir(dir)?
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.starts_with("deploy-") && n.ends_with(".log"))
        })
        .collect();
    old.sort();
    for p in old.iter().take(old.len().saturating_sub(LOGS_KEPT - 1)) {
        let _ = std::fs::remove_file(p);
    }
    Ok(dir.join(format!("deploy-{:015}.log", now_ms())))
}

/// Starts HITMAN. Steam installs start through Steam, as its own Play
/// button does; other stores are not supported yet.
pub fn launch(game: &GameInstall) -> Result<(), VmmError> {
    match game.store {
        Store::Steam => crate::host::open(&format!("steam://rungameid/{STEAM_APP_ID}")),
        other => Err(VmmError::new(
            ErrorCode::LaunchUnsupported,
            format!(
                "Play starts HITMAN through Steam, and this copy is from {}. Start HITMAN from there; the deploy is done.",
                other.label()
            ),
        )),
    }
}

/// What the deploy line says, and whether Deploy and Play can be used.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Status {
    pub text: String,
    /// "busy", "ok", "warn" or "error".
    pub tone: &'static str,
    pub can_deploy: bool,
    pub can_play: bool,
    pub has_log: bool,
}

pub fn status(running: bool, last_line: &str, record: &Record, current: Option<&str>, blocker: Option<&str>) -> Status {
    let has_log = record.log.is_some();
    if running {
        return Status {
            text: format!("Deploying… {last_line}"),
            tone: "busy",
            can_deploy: false,
            can_play: false,
            has_log,
        };
    }
    let (text, tone) = if record.success == Some(false) {
        let code = record.exit_code.map_or_else(|| "it was stopped".to_string(), |c| format!("exit code {c}"));
        (format!("The last deploy failed ({code}). Open the log to see why."), "error")
    } else if record.fingerprint.is_some() && record.fingerprint.as_deref() == current {
        ("Deployed and up to date.".to_string(), "ok")
    } else if record.success == Some(true) {
        ("Changes since the last deploy. Deploy, or Play, to put them in the game.".to_string(), "warn")
    } else {
        ("Not deployed from Vaexil Mod Manager yet.".to_string(), "warn")
    };
    match blocker {
        Some(b) => Status { text: format!("{text} {b}"), tone, can_deploy: false, can_play: false, has_log },
        None => Status { text, tone, can_deploy: true, can_play: true, has_log },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn says_what_state_the_game_is_in() {
        let fresh = Record::default();
        assert_eq!(status(false, "", &fresh, Some("f1"), None).text, "Not deployed from Vaexil Mod Manager yet.");
        let done = Record { fingerprint: Some("f1".into()), success: Some(true), ..Record::default() };
        let s = status(false, "", &done, Some("f1"), None);
        assert_eq!((s.text.as_str(), s.tone, s.can_play), ("Deployed and up to date.", "ok", true));
        assert_eq!(status(false, "", &done, Some("f2"), None).tone, "warn");
        let failed =
            Record { success: Some(false), exit_code: Some(1), log: Some("x.log".into()), ..Record::default() };
        let s = status(false, "", &failed, Some("f1"), None);
        assert!(s.text.contains("exit code 1") && s.has_log);
        let busy = status(true, "Staging RPKG mod", &done, Some("f1"), None);
        assert_eq!((busy.tone, busy.can_deploy), ("busy", false));
        assert!(busy.text.contains("Staging RPKG mod"));
        let b = blocked(&["hitman3.exe".into()]);
        assert_eq!(b.as_ref().map(|e| e.code), Some(ErrorCode::GameRunning));
        let s = status(false, "", &done, Some("f1"), b.as_ref().map(|e| e.message.as_str()));
        assert!(!s.can_deploy && !s.can_play && s.text.contains("HITMAN is running"));
        let d = blocked(&["Deploy.exe".into()]).unwrap();
        assert!(d.code == ErrorCode::DeployRunning && d.message.contains("already running"));
        assert!(blocked(&[]).is_none());
    }

    #[test]
    fn the_record_round_trips_and_logs_are_pruned() {
        let tmp = tempfile::tempdir().unwrap();
        let r = Record {
            fingerprint: Some("abc".into()),
            success: Some(true),
            exit_code: Some(0),
            log: Some("l.log".into()),
            at_ms: Some(5),
        };
        r.save(tmp.path()).unwrap();
        assert_eq!(Record::load(tmp.path()), r);
        let logs = tmp.path().join("logs");
        std::fs::create_dir_all(&logs).unwrap();
        for i in 0..LOGS_KEPT + 3 {
            std::fs::write(logs.join(format!("deploy-{i:015}.log")), "").unwrap();
        }
        let next = new_log(&logs).unwrap();
        std::fs::write(&next, "").unwrap();
        assert_eq!(std::fs::read_dir(&logs).unwrap().count(), LOGS_KEPT);
    }

    #[test]
    fn play_is_for_steam_copies() {
        let epic = GameInstall { store: Store::Epic, root: PathBuf::from("x") };
        assert!(launch(&epic).unwrap_err().message.contains("Epic Games"));
    }
}
