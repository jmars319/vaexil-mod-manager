//! Keeping Vaexil Mod Manager itself current (ADR 0003, E8).
//!
//! A published release puts its installer in JAMARQ-Digital-LLC/jamarq-releases
//! and points `vaexil-mod-manager-latest/release.json` (version, url, sha256) at
//! it. The app reads that manifest through Windows' own WinHTTP, at most once a
//! day when the window opens on a real install, or when asked. Updating, which
//! the user confirms, downloads the installer, checks it against the
//! manifest's SHA-256, and runs it silently. The installer sets aside files
//! that are in use (an AI app's session runs `vmm.exe`), so nothing has to be
//! closed first. Never during a deploy or an SMF update.
//!
//! The installers are unsigned (owner's decision), so the SHA-256 from the
//! manifest, and a URL inside the feed's own releases, are what make a
//! download trustworthy.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use vmm_core::error::{ErrorCode, VmmError};

use crate::{Engine, deploy, http};

/// Vaexil Mod Manager's manifest in the shared release feed.
pub const FEED: &str =
    "https://github.com/JAMARQ-Digital-LLC/jamarq-releases/releases/download/vaexil-mod-manager-latest/release.json";

/// Where an installer may come from: the feed's own releases only.
const TRUSTED: &str = "https://github.com/JAMARQ-Digital-LLC/jamarq-releases/releases/download/vaexil-mod-manager-v";

/// The last check, kept in the data folder.
pub const LAST_CHECK: &str = "app-update.json";

/// Where installers are downloaded.
const DOWNLOADS: &str = "app-updates";

/// This build's version.
pub const CURRENT: &str = env!("CARGO_PKG_VERSION");

/// The feed's manifest, as jamarq-releases' `publish/feed.mjs` writes it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Release {
    pub version: String,
    /// The installer.
    pub url: String,
    pub sha256: String,
    #[serde(default)]
    pub notes: Option<String>,
}

/// What the last check found.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LastCheck {
    pub checked_at_ms: u64,
    /// The version that checked.
    pub current: String,
    /// A newer release, if there was one.
    pub latest: Option<Release>,
}

/// MAJOR.MINOR.PATCH as numbers, or `None`.
fn version(v: &str) -> Option<[u64; 3]> {
    let parts: Vec<u64> = v.trim().split('.').map(|p| p.parse().ok()).collect::<Option<_>>()?;
    <[u64; 3]>::try_from(parts).ok()
}

/// Whether `candidate` is a newer version than `current`.
pub fn is_newer(candidate: &str, current: &str) -> bool {
    matches!((version(candidate), version(current)), (Some(a), Some(b)) if a > b)
}

/// Reads a manifest, refusing one whose installer isn't in the feed's own
/// releases, whose version isn't MAJOR.MINOR.PATCH, or whose SHA-256 is
/// missing.
pub fn parse(text: &str) -> Result<Release, VmmError> {
    let bad = |why: String| VmmError::new(ErrorCode::NetworkFailed, format!("The update feed can't be used: {why}."));
    let r: Release = serde_json::from_str(text).map_err(|e| bad(format!("it isn't a release manifest ({e})")))?;
    if version(&r.version).is_none() {
        return Err(bad(format!("it names version \u{201c}{}\u{201d}", r.version)));
    }
    if !r.url.starts_with(TRUSTED) || !r.url.ends_with("-setup.exe") {
        return Err(bad("it points outside Vaexil Mod Manager's own releases".into()));
    }
    if r.sha256.len() != 64 || !r.sha256.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(bad("it gives no SHA-256 for the installer".into()));
    }
    Ok(r)
}

pub fn last_check(data: &Path) -> Option<LastCheck> {
    serde_json::from_slice(&std::fs::read(data.join(LAST_CHECK)).ok()?).ok()
}

/// Reads the feed and remembers the answer. A newer release is `Some`. Before
/// the first release the feed doesn't exist, which is nothing newer. No
/// engine needed, so the window can do it on a worker thread.
pub fn check(data: &Path) -> Result<Option<Release>, VmmError> {
    check_with(data, CURRENT, || http::get(FEED, "application/json"))
}

fn check_with(
    data: &Path,
    current: &str,
    fetch: impl FnOnce() -> Result<Vec<u8>, VmmError>,
) -> Result<Option<Release>, VmmError> {
    let latest = match fetch() {
        Ok(body) => Some(parse(&String::from_utf8_lossy(&body))?).filter(|r| is_newer(&r.version, current)),
        Err(e) if e.message.contains("status 404") => None,
        Err(e) => return Err(e),
    };
    let last = LastCheck { checked_at_ms: deploy::now_ms(), current: current.to_string(), latest: latest.clone() };
    let _ = std::fs::create_dir_all(data);
    let _ = std::fs::write(data.join(LAST_CHECK), serde_json::to_vec_pretty(&last).expect("serializes"));
    Ok(latest)
}

/// The last check's newer release, if it is still newer than this build.
pub fn known_newer(data: &Path) -> Option<Release> {
    last_check(data)?.latest.filter(|r| is_newer(&r.version, CURRENT))
}

/// Downloads `release`'s installer into the data folder and checks its
/// SHA-256. A download that doesn't match is deleted. `progress` gets a line
/// now and then.
pub fn download(data: &Path, release: &Release, progress: &mut dyn FnMut(String)) -> Result<PathBuf, VmmError> {
    let dir = data.join(DOWNLOADS);
    let save = |e: std::io::Error| VmmError::new(ErrorCode::SaveFailed, format!("Couldn't prepare the update: {e}"));
    std::fs::create_dir_all(&dir).map_err(save)?;
    // Only this release's installer is kept.
    let name = format!("VaexilModManager-{}-setup.exe", release.version);
    for e in std::fs::read_dir(&dir).map_err(save)?.flatten() {
        if e.file_name().to_string_lossy() != name {
            let _ = std::fs::remove_file(e.path());
        }
    }
    let installer = dir.join(&name);
    let matches = |p: &Path| vmm_backend_smf::sha256_file(p).is_ok_and(|h| h.eq_ignore_ascii_case(&release.sha256));
    if !(installer.is_file() && matches(&installer)) {
        let mut last_pct = u64::MAX;
        http::download(&release.url, &installer, &mut |done, total| {
            let pct = total.filter(|t| *t > 0).map_or(0, |t| done * 100 / t);
            if pct != last_pct {
                last_pct = pct;
                progress(format!("Downloading Vaexil Mod Manager {}: {pct}%", release.version));
            }
        })?;
        progress("Checking the download".into());
        if !matches(&installer) {
            let _ = std::fs::remove_file(&installer);
            return Err(VmmError::new(
                ErrorCode::DownloadMismatch,
                "The download doesn't match the SHA-256 the update feed gives, so it was deleted. Nothing was installed.",
            ));
        }
    }
    Ok(installer)
}

/// The installer's arguments: silent, and with `relaunch`, reopening the app
/// afterwards (`/relaunch=1`, read by packaging/installer.iss).
pub fn installer_args(relaunch: bool) -> Vec<&'static str> {
    let mut args = vec!["/VERYSILENT", "/SUPPRESSMSGBOXES", "/NORESTART"];
    if relaunch {
        args.push("/relaunch=1");
    }
    args
}

impl Engine {
    /// What the last check of the release feed found.
    pub fn app_update_status(&self) -> vmm_core::schema::AppUpdateStatus {
        let last = self.data_dir().and_then(last_check);
        let latest = last.as_ref().and_then(|l| l.latest.clone()).filter(|r| is_newer(&r.version, CURRENT));
        vmm_core::schema::AppUpdateStatus {
            schema_version: vmm_core::schema::SCHEMA_VERSION,
            current: CURRENT.to_string(),
            latest: latest.as_ref().map(|r| r.version.clone()),
            url: latest.map(|r| r.url),
            checked_at_ms: last.map(|l| l.checked_at_ms),
        }
    }

    /// Refuses while a deploy or an SMF update runs: the window closes for
    /// the update, and the CLI's `vmm.exe` is replaced.
    pub fn check_can_update_app(&self) -> Result<(), VmmError> {
        if self.deploy_progress().0 {
            return Err(VmmError::new(ErrorCode::DeployRunning, "A deploy is running. Update when it has finished."));
        }
        if self.data_dir().is_some_and(crate::smf_update::updating) {
            return Err(VmmError::new(ErrorCode::SmfUpdating, "SMF is being updated. Update when it has finished."));
        }
        if !crate::host::running(&["Deploy.exe"]).is_empty() {
            return Err(VmmError::new(ErrorCode::DeployRunning, "Deploy.exe is running. Update when it has finished."));
        }
        Ok(())
    }

    /// Records an update attempt in the event log.
    pub fn note_app_update(&self, r: &Result<String, VmmError>) {
        use vmm_core::schema::LogLevel;
        match r {
            Ok(m) => self.log(LogLevel::Info, "app.update", None, None, m.clone(), None),
            Err(e) => self.log(LogLevel::Error, "app.update", None, None, e.message.clone(), Some(e)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const GOOD: &str = r#"{"version":"0.2.0","notes":"Vaexil Mod Manager 0.2.0.","pub_date":"2026-11-14T00:00:00Z","url":"https://github.com/JAMARQ-Digital-LLC/jamarq-releases/releases/download/vaexil-mod-manager-v0.2.0/vaexil-mod-manager-0.2.0-x64-setup.exe","sha256":"fd067473167d934797660c6e40d45f6df8417eb6adee74e039af2a0e63d07765"}"#;

    #[test]
    fn versions_compare_as_numbers() {
        assert!(is_newer("0.2.0", "0.1.0"));
        assert!(is_newer("0.10.0", "0.9.9"));
        assert!(!is_newer("0.2.0", "0.2.0"));
        assert!(!is_newer("0.1.9", "0.2.0"));
        assert!(!is_newer("0.2", "0.1.0"), "not MAJOR.MINOR.PATCH");
    }

    #[test]
    fn only_a_manifest_for_our_own_releases_is_used() {
        assert_eq!(parse(GOOD).unwrap().version, "0.2.0");
        let elsewhere = GOOD.replace("JAMARQ-Digital-LLC/jamarq-releases", "someone/else");
        assert!(parse(&elsewhere).unwrap_err().message.contains("outside"));
        let other_app = GOOD.replace("vaexil-mod-manager-v0.2.0", "keeplight-v0.2.0");
        assert!(parse(&other_app).is_err());
        assert!(parse(&GOOD.replace("fd0674", "zz0674")).unwrap_err().message.contains("SHA-256"));
        assert!(parse(&GOOD.replace("\"0.2.0\",\"notes", "\"0.2\",\"notes")).is_err());
        assert!(parse("<html>").is_err());
    }

    #[test]
    fn a_check_remembers_a_newer_release_and_a_missing_feed_is_nothing_newer() {
        let tmp = tempfile::tempdir().unwrap();
        let found = check_with(tmp.path(), "0.1.0", || Ok(GOOD.as_bytes().to_vec())).unwrap();
        assert_eq!(found.unwrap().version, "0.2.0");
        assert_eq!(last_check(tmp.path()).unwrap().latest.unwrap().version, "0.2.0");
        assert_eq!(check_with(tmp.path(), "0.2.0", || Ok(GOOD.as_bytes().to_vec())).unwrap(), None);
        assert_eq!(last_check(tmp.path()).unwrap().latest, None);
        let missing = || Err(VmmError::new(ErrorCode::NetworkFailed, "github.com answered with HTTP status 404"));
        assert_eq!(check_with(tmp.path(), "0.1.0", missing).unwrap(), None, "no release yet");
        let down = || Err(VmmError::new(ErrorCode::NetworkFailed, "github.com answered with HTTP status 500"));
        assert!(check_with(tmp.path(), "0.1.0", down).is_err());
    }

    #[test]
    fn the_installer_runs_silently_and_reopens_only_when_asked() {
        assert_eq!(installer_args(false), ["/VERYSILENT", "/SUPPRESSMSGBOXES", "/NORESTART"]);
        assert_eq!(installer_args(true).last(), Some(&"/relaunch=1"));
    }
}
