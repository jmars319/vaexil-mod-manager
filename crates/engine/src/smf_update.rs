//! Keeping SMF current (ADR 0003, E7). Checking reads SMF's release list on
//! GitHub; updating downloads the newest `Release.zip`, checks it against the
//! SHA-256 GitHub gives, and applies it the way SMF's own updater does
//! (`vmm_backend_smf::update`), keeping a copy of every file it replaces.
//! Nothing happens while HITMAN, a deploy or SMF's own Mod Manager runs.
//!
//! An update is a job of plain data, so the window can run it on a worker
//! thread while it shows progress; the command line runs it in place.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use vmm_backend_smf::update::{self, Release};
use vmm_core::backend::{Status, Version};
use vmm_core::error::{ErrorCode, VmmError};
use vmm_core::game::GameInstall;
use vmm_core::schema::LogLevel;

use crate::{Engine, deploy, host, http};

/// The last check, kept so the window can say an update is available
/// without asking GitHub every time.
const LAST_CHECK: &str = "smf-update.json";
/// SMF versions this app has installed, by `Deploy.exe`'s SHA-256, so they
/// are known even when the built-in table doesn't list them.
pub const LEARNED: &str = "smf-versions.json";
/// How many SMF backups are kept.
const BACKUPS_KEPT: usize = 2;
/// How often the window checks by itself.
pub const CHECK_EVERY_MS: u64 = 24 * 60 * 60 * 1000;

/// Processes that stop an update: the game, a deploy, SMF's own Mod Manager.
const BLOCKERS: [&str; 3] = ["HITMAN3.exe", "Deploy.exe", "Mod Manager.exe"];

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LastCheck {
    pub checked_at_ms: u64,
    pub installed: String,
    /// The newest newer release, if any.
    pub latest: Option<String>,
    pub held: bool,
    pub size: u64,
}

/// An update ready to run: everything it needs, as plain data.
#[derive(Debug, Clone)]
pub struct UpdateJob {
    pub installed: String,
    pub release: Release,
    pub smf_folder: PathBuf,
    pub data_dir: PathBuf,
}

fn last_check_path(data: &Path) -> PathBuf {
    data.join(LAST_CHECK)
}

/// Reads the last check, if any.
pub fn last_check(data: &Path) -> Option<LastCheck> {
    serde_json::from_slice(&std::fs::read(last_check_path(data)).ok()?).ok()
}

/// Asks GitHub which releases are newer than `installed`, and remembers the
/// answer. No engine needed, so the window can do it on a worker thread.
pub fn check(data: &Path, installed: &str) -> Result<(LastCheck, update::UpdateCheck), VmmError> {
    let body = http::get(update::RELEASES_URL, "application/vnd.github+json")?;
    let text = String::from_utf8_lossy(&body);
    let found = update::read_releases(&text, installed)?;
    let last = LastCheck {
        checked_at_ms: deploy::now_ms(),
        installed: installed.to_string(),
        latest: found.latest.as_ref().map(|r| r.tag.clone()),
        held: found.held,
        size: found.latest.as_ref().map_or(0, |r| r.size),
    };
    let _ = std::fs::create_dir_all(data);
    let _ = std::fs::write(last_check_path(data), serde_json::to_vec_pretty(&last).expect("serializes"));
    Ok((last, found))
}

/// Downloads, checks, and applies the update. `progress` gets a line for
/// people as it goes.
pub fn run(job: &UpdateJob, progress: &mut dyn FnMut(String)) -> Result<String, VmmError> {
    let want = job.release.sha256.clone().ok_or_else(|| {
        VmmError::new(
            ErrorCode::DownloadMismatch,
            format!(
                "GitHub gives no checksum for SMF {}, so it can't be checked. Nothing was installed.",
                job.release.tag
            ),
        )
    })?;
    let dir = job.data_dir.join("smf-updates");
    let save = |e: std::io::Error| VmmError::new(ErrorCode::SaveFailed, format!("Couldn't prepare the update: {e}"));
    std::fs::create_dir_all(&dir).map_err(save)?;
    let zip = dir.join(format!("Release-{}.zip", job.release.tag));
    let matches = |p: &Path| vmm_backend_smf::sha256_file(p).is_ok_and(|h| h.eq_ignore_ascii_case(&want));
    if !(zip.is_file() && matches(&zip)) {
        let mut last_pct = u64::MAX;
        http::download(&job.release.zip_url, &zip, &mut |done, total| {
            let pct = total.filter(|t| *t > 0).map_or(0, |t| done * 100 / t);
            if pct != last_pct {
                last_pct = pct;
                progress(format!("Downloading SMF {}: {pct}% of {} MB", job.release.tag, total.unwrap_or(0) >> 20));
            }
        })?;
        progress("Checking the download".into());
        if !matches(&zip) {
            let _ = std::fs::remove_file(&zip);
            return Err(VmmError::new(
                ErrorCode::DownloadMismatch,
                "The download doesn't match the checksum GitHub gives, so it was deleted. Nothing was installed.",
            ));
        }
    }
    progress("Unpacking".into());
    let staging = dir.join("staging");
    update::extract(&zip, &staging)?;
    update::prune(&staging)?;
    let backups = job.data_dir.join("smf-backups");
    let backup = backups.join(format!("{}-{:015}", job.installed, deploy::now_ms()));
    progress("Installing (a copy of every replaced file is kept)".into());
    let count = update::apply(&staging, &job.smf_folder, &backup)?;
    let _ = std::fs::remove_dir_all(&staging);
    let _ = std::fs::remove_file(&zip);
    prune_backups(&backups);
    // Remember which Deploy.exe this release has, so it's known by version.
    if let Ok(sha) = vmm_backend_smf::sha256_file(&job.smf_folder.join("Deploy.exe")) {
        learn(&job.data_dir, &sha, &job.release.tag);
    }
    let _ = std::fs::write(
        last_check_path(&job.data_dir),
        serde_json::to_vec_pretty(&LastCheck {
            checked_at_ms: deploy::now_ms(),
            installed: job.release.tag.clone(),
            ..LastCheck::default()
        })
        .expect("serializes"),
    );
    Ok(format!(
        "SMF is now {} ({count} files updated). Your mods and SMF's settings were kept, and a copy of the old files is in {}. Deploy to use it.",
        job.release.tag,
        backup.display()
    ))
}

fn prune_backups(dir: &Path) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    let mut all: Vec<(String, PathBuf)> = rd
        .flatten()
        .filter(|e| e.path().is_dir())
        .map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            // Sort by the time after the last '-'.
            (name.rsplit('-').next().unwrap_or_default().to_string(), e.path())
        })
        .collect();
    all.sort();
    for (_, p) in all.iter().take(all.len().saturating_sub(BACKUPS_KEPT)) {
        let _ = std::fs::remove_dir_all(p);
    }
}

/// Every version this app has installed, by `Deploy.exe`'s SHA-256.
pub fn learned(data: &Path) -> BTreeMap<String, String> {
    std::fs::read(data.join(LEARNED)).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
}

fn learn(data: &Path, sha: &str, tag: &str) {
    let mut all = learned(data);
    all.insert(sha.to_lowercase(), tag.to_string());
    let _ = std::fs::write(data.join(LEARNED), serde_json::to_vec_pretty(&all).expect("serializes"));
}

impl Engine {
    /// The installed SMF version as a version number, if it is one this app
    /// knows (from its table, or because this app installed it).
    pub fn smf_version(&self, game: &GameInstall) -> Option<String> {
        // The backend already names versions this app installed (LEARNED).
        match self.inspect(game).status {
            Status::Installed { version: Version::Known(v), .. } => Some(v),
            _ => None,
        }
    }

    /// What the last check found, without going online.
    pub fn smf_update_status(&self, game: Option<&GameInstall>) -> vmm_core::schema::SmfUpdateStatus {
        let installed = game.and_then(|g| self.smf_version(g));
        let last = self.data_dir().and_then(last_check);
        // A check made for another installed version says nothing now.
        let last = last.filter(|l| installed.as_deref() == Some(l.installed.as_str()));
        vmm_core::schema::SmfUpdateStatus {
            schema_version: vmm_core::schema::SCHEMA_VERSION,
            installed,
            latest: last.as_ref().and_then(|l| l.latest.clone()),
            held: last.as_ref().is_some_and(|l| l.held),
            size_bytes: last.as_ref().map_or(0, |l| l.size),
            checked_at_ms: last.as_ref().map(|l| l.checked_at_ms),
        }
    }

    /// Asks GitHub for SMF's releases now.
    pub fn check_smf_update(&self, game: &GameInstall) -> Result<LastCheck, VmmError> {
        let data = self.data_dir().ok_or_else(|| {
            VmmError::new(ErrorCode::BackupUnavailable, "There is no folder to keep the update check in.")
        })?;
        let installed = self.smf_version(game).ok_or_else(|| {
            VmmError::new(
                ErrorCode::SmfUpdateHeld,
                "The installed SMF isn't a release this app can name, so it can't tell what is newer.",
            )
        })?;
        let r = check(data, &installed).map(|(last, _)| last);
        match &r {
            Ok(l) => self.log(
                LogLevel::Info,
                "smf.checked",
                Some(game),
                None,
                match &l.latest {
                    Some(t) => format!("SMF {t} is available (installed {installed})."),
                    None => format!("SMF {installed} is the newest."),
                },
                None,
            ),
            Err(e) => self.log(LogLevel::Error, "smf.checked", Some(game), None, e.message.clone(), Some(e)),
        }
        r
    }

    /// Checks it is safe to update now, asks GitHub, and returns the job, or
    /// `None` when SMF is already the newest.
    pub fn prepare_smf_update(&self, game: &GameInstall) -> Result<Option<UpdateJob>, VmmError> {
        let running = host::running(&BLOCKERS);
        if let Some(name) = running.first() {
            return Err(VmmError::new(
                if name.eq_ignore_ascii_case("HITMAN3.exe") {
                    ErrorCode::GameRunning
                } else {
                    ErrorCode::DeployRunning
                },
                format!("{name} is running. Close it, then update SMF. Nothing was changed."),
            ));
        }
        if self.deploy_progress().0 {
            return Err(VmmError::new(ErrorCode::DeployRunning, "A deploy is running. Nothing was changed."));
        }
        let data = self.data_dir().ok_or_else(|| {
            VmmError::new(ErrorCode::BackupUnavailable, "There is no folder to keep a copy of SMF's files in.")
        })?;
        let smf_folder = vmm_backend_smf::find_folder(&game.root)
            .ok_or_else(|| VmmError::new(ErrorCode::FrameworkMissing, "SMF isn't in the game folder."))?;
        let installed = self.smf_version(game).ok_or_else(|| {
            VmmError::new(
                ErrorCode::SmfUpdateHeld,
                "The installed SMF isn't a release this app can name, so it won't replace it automatically.",
            )
        })?;
        let (_, found) = check(data, &installed)?;
        if found.held {
            return Err(VmmError::new(
                ErrorCode::SmfUpdateHeld,
                format!(
                    "SMF's release notes say a newer release ({}) can't be applied automatically. Nothing was changed.",
                    found.newer.join(", ")
                ),
            ));
        }
        Ok(found.latest.map(|release| UpdateJob { installed, release, smf_folder, data_dir: data.to_path_buf() }))
    }

    /// Records how an update ended, as the window and `vmm` do once it has.
    pub fn note_smf_update(&self, game: &GameInstall, r: &Result<String, VmmError>) {
        match r {
            Ok(m) => self.log(LogLevel::Info, "smf.updated", Some(game), None, m.clone(), None),
            Err(e) => self.log(LogLevel::Error, "smf.updated", Some(game), None, e.message.clone(), Some(e)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn learned_versions_round_trip_and_old_backups_go() {
        let tmp = tempfile::tempdir().unwrap();
        learn(tmp.path(), "ABC", "2.33.46");
        assert_eq!(learned(tmp.path()).get("abc").map(String::as_str), Some("2.33.46"));
        let b = tmp.path().join("smf-backups");
        for (i, v) in ["2.33.43", "2.33.44", "2.33.45"].iter().enumerate() {
            std::fs::create_dir_all(b.join(format!("{v}-{i:015}"))).unwrap();
        }
        prune_backups(&b);
        let mut left: Vec<String> =
            std::fs::read_dir(&b).unwrap().flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect();
        left.sort();
        assert_eq!(left, ["2.33.44-000000000000001", "2.33.45-000000000000002"]);
    }
}
