//! Keeping SMF current the way SMF's own updater does (`startFrameworkUpdate`
//! in `Mod Manager/src/routes/+page.svelte`, SMF 2.33.45), so an update made
//! here leaves SMF's folder exactly as SMF's updater would:
//!
//! - A release newer than the installed one is offered unless the notes of
//!   any newer release say "CANNOT AUTOMATICALLY UPDATE".
//! - Its `Release.zip` is extracted to a staging folder, from which the files
//!   SMF keeps are removed first: `Mods`, `config.json`,
//!   `cleanPackageDefinition.txt`, `cleanThumbs.dat`, and the Mod Manager's
//!   Chromium runtime.
//! - The old `Load Order Manager` folder is removed, and the staging folder is
//!   copied over SMF's folder.
//!
//! Unlike SMF's updater, every file that is replaced or removed is copied
//! to a backup folder first, and a failed copy puts everything back.

use std::path::{Path, PathBuf};

use vmm_core::error::{ErrorCode, VmmError};

/// What SMF's release notes say when an update can't be applied by copying.
pub const HELD_MARKER: &str = "CANNOT AUTOMATICALLY UPDATE";
/// The release asset SMF's updater downloads.
pub const ASSET: &str = "Release.zip";
/// Where SMF's releases are listed.
pub const RELEASES_URL: &str = "https://api.github.com/repos/atampy25/simple-mod-framework/releases?per_page=30";

/// Paths (relative to SMF's folder) that an update never overwrites.
const KEPT: &[&str] = &[
    "Mods",
    "cleanPackageDefinition.txt",
    "cleanThumbs.dat",
    "config.json",
    "Mod Manager/chrome_100_percent.pak",
    "Mod Manager/chrome_200_percent.pak",
    "Mod Manager/d3dcompiler_47.dll",
    "Mod Manager/ffmpeg.dll",
    "Mod Manager/icudtl.dat",
    "Mod Manager/libEGL.dll",
    "Mod Manager/libGLESv2.dll",
    "Mod Manager/Mod Manager.exe",
    "Mod Manager/locales",
    "Mod Manager/resources.pak",
    "Mod Manager/snapshot_blob.bin",
    "Mod Manager/v8_context_snapshot.bin",
    "Mod Manager/vk_swiftshader.dll",
    "Mod Manager/vk_swiftshader_icd.json",
    "Mod Manager/vulkan_1.dll",
];

/// The folder SMF's updater deletes.
const REMOVED: &str = "Load Order Manager";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Release {
    pub tag: String,
    pub zip_url: String,
    /// The zip's SHA-256 as GitHub reports it, lowercase hex.
    pub sha256: Option<String>,
    pub size: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdateCheck {
    /// The newest release, when it is newer than the installed one.
    pub latest: Option<Release>,
    /// Every newer release's tag, newest first.
    pub newer: Vec<String>,
    /// Whether a newer release says it can't be applied automatically.
    pub held: bool,
}

fn parse_version(tag: &str) -> Option<semver::Version> {
    semver::Version::parse(tag.trim().trim_start_matches('v')).ok()
}

/// Reads GitHub's release list (JSON) against the installed version.
pub fn read_releases(json: &str, installed: &str) -> Result<UpdateCheck, VmmError> {
    let bad =
        |why: String| VmmError::new(ErrorCode::NetworkFailed, format!("GitHub's release list can't be read: {why}"));
    let installed = parse_version(installed).ok_or_else(|| {
        VmmError::new(
            ErrorCode::SmfUpdateHeld,
            format!("The installed SMF version, {installed}, isn't one this app can compare."),
        )
    })?;
    let list: Vec<serde_json::Value> = serde_json::from_str(json).map_err(|e| bad(e.to_string()))?;
    // The list is the newest releases only. If it doesn't reach back to the
    // installed one, a release in between may say it can't be applied
    // automatically, so the update is held.
    let covered = list.iter().filter_map(|r| parse_version(r["tag_name"].as_str()?)).any(|v| v <= installed);
    let mut newer: Vec<(semver::Version, &serde_json::Value)> = list
        .iter()
        .filter(|r| !r["draft"].as_bool().unwrap_or(false) && !r["prerelease"].as_bool().unwrap_or(false))
        .filter_map(|r| Some((parse_version(r["tag_name"].as_str()?)?, r)))
        .filter(|(v, _)| *v > installed)
        .collect();
    newer.sort_by(|a, b| b.0.cmp(&a.0));
    let held = (!covered && !newer.is_empty())
        || newer.iter().any(|(_, r)| r["body"].as_str().unwrap_or_default().contains(HELD_MARKER));
    let latest = match newer.first() {
        None => None,
        Some((_, r)) => {
            let asset = r["assets"]
                .as_array()
                .and_then(|a| a.iter().find(|x| x["name"].as_str() == Some(ASSET)))
                .ok_or_else(|| bad(format!("release {} has no {ASSET}", r["tag_name"])))?;
            Some(Release {
                tag: r["tag_name"].as_str().unwrap_or_default().to_string(),
                zip_url: asset["browser_download_url"].as_str().unwrap_or_default().to_string(),
                sha256: asset["digest"].as_str().and_then(|d| d.strip_prefix("sha256:")).map(str::to_lowercase),
                size: asset["size"].as_u64().unwrap_or(0),
            })
        }
    };
    Ok(UpdateCheck {
        latest,
        newer: newer.iter().map(|(_, r)| r["tag_name"].as_str().unwrap_or_default().to_string()).collect(),
        held,
    })
}

fn io(context: &str) -> impl Fn(std::io::Error) -> VmmError + '_ {
    move |e| VmmError::new(ErrorCode::Io, format!("{context}: {e}"))
}

/// Extracts `zip` into `staging` (emptied first). Entries that would land
/// outside `staging` are refused.
pub fn extract(zip: &Path, staging: &Path) -> Result<(), VmmError> {
    if staging.exists() {
        std::fs::remove_dir_all(staging).map_err(io("Couldn't empty the staging folder"))?;
    }
    std::fs::create_dir_all(staging).map_err(io("Couldn't make the staging folder"))?;
    let file = std::fs::File::open(zip).map_err(io("Couldn't open the download"))?;
    let mut archive = zip::ZipArchive::new(file)
        .map_err(|e| VmmError::new(ErrorCode::DownloadMismatch, format!("The download isn't a readable zip: {e}")))?;
    for i in 0..archive.len() {
        let mut entry = archive
            .by_index(i)
            .map_err(|e| VmmError::new(ErrorCode::DownloadMismatch, format!("A file in the zip can't be read: {e}")))?;
        let Some(rel) = entry.enclosed_name() else {
            return Err(VmmError::new(
                ErrorCode::DownloadMismatch,
                format!(
                    "The zip has an entry outside its own folder ({}), so it wasn't used.",
                    entry.name().unwrap_or_default()
                ),
            ));
        };
        let out = staging.join(rel);
        if entry.is_dir() {
            std::fs::create_dir_all(&out).map_err(io("Couldn't extract"))?;
            continue;
        }
        if let Some(parent) = out.parent() {
            std::fs::create_dir_all(parent).map_err(io("Couldn't extract"))?;
        }
        let mut f = std::fs::File::create(&out).map_err(io("Couldn't extract"))?;
        std::io::copy(&mut entry, &mut f).map_err(io("Couldn't extract"))?;
    }
    Ok(())
}

/// Removes from `staging` what an update never overwrites, as SMF's updater
/// does.
pub fn prune(staging: &Path) -> Result<(), VmmError> {
    for rel in KEPT {
        let p = staging.join(rel);
        if p.is_dir() {
            std::fs::remove_dir_all(&p).map_err(io("Couldn't prepare the update"))?;
        } else if p.exists() {
            std::fs::remove_file(&p).map_err(io("Couldn't prepare the update"))?;
        }
    }
    Ok(())
}

/// Every file under `dir`, as paths relative to it.
fn files(dir: &Path) -> Result<Vec<PathBuf>, VmmError> {
    let mut out = Vec::new();
    let mut stack = vec![PathBuf::new()];
    while let Some(rel) = stack.pop() {
        for e in std::fs::read_dir(dir.join(&rel)).map_err(io("Couldn't read the update"))?.flatten() {
            let r = rel.join(e.file_name());
            if e.path().is_dir() { stack.push(r) } else { out.push(r) }
        }
    }
    out.sort();
    Ok(out)
}

/// Copies `staging` over SMF's `folder`. Each file that will be replaced, and
/// the `Load Order Manager` folder SMF's updater removes, is copied to
/// `backup` first. If anything fails, the files are put back as they were.
pub fn apply(staging: &Path, folder: &Path, backup: &Path) -> Result<usize, VmmError> {
    let incoming = files(staging)?;
    std::fs::create_dir_all(backup).map_err(io("Couldn't make the backup folder"))?;
    let copy = |from: &Path, to: &Path| -> std::io::Result<()> {
        if let Some(parent) = to.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::copy(from, to).map(|_| ())
    };
    // Back up first, so a failure while backing up changes nothing.
    for rel in &incoming {
        let existing = folder.join(rel);
        if existing.is_file() {
            copy(&existing, &backup.join(rel))
                .map_err(io("Couldn't back up SMF before updating; nothing was changed"))?;
        }
    }
    let lom = folder.join(REMOVED);
    if lom.is_dir() {
        for rel in files(&lom)? {
            copy(&lom.join(&rel), &backup.join(REMOVED).join(&rel))
                .map_err(io("Couldn't back up SMF before updating; nothing was changed"))?;
        }
    }
    let mut written: Vec<&PathBuf> = Vec::new();
    for rel in &incoming {
        if let Err(e) = copy(&staging.join(rel), &folder.join(rel)) {
            // Put back what was replaced, and remove what was new, including
            // the file whose copy failed partway.
            written.push(rel);
            for done in &written {
                let saved = backup.join(done);
                let _ = if saved.is_file() {
                    copy(&saved, &folder.join(done))
                } else {
                    std::fs::remove_file(folder.join(done))
                };
            }
            return Err(VmmError::new(
                ErrorCode::SaveFailed,
                format!("Couldn't update {} ({e}). SMF was put back as it was.", rel.display()),
            ));
        }
        written.push(rel);
    }
    if lom.is_dir() {
        let _ = std::fs::remove_dir_all(&lom);
    }
    Ok(incoming.len())
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use super::*;

    fn release(tag: &str, body: &str) -> serde_json::Value {
        serde_json::json!({
            "tag_name": tag, "draft": false, "prerelease": false, "body": body,
            "assets": [{ "name": "Release.zip", "size": 10, "digest": "sha256:ABC",
                         "browser_download_url": format!("https://example.invalid/{tag}/Release.zip") }]
        })
    }

    #[test]
    fn offers_the_newest_newer_release_and_honours_the_hold() {
        let list = serde_json::json!([release("2.33.46", "fixes"), release("2.33.45", ""), release("2.33.44", "")]);
        let c = read_releases(&list.to_string(), "2.33.44").unwrap();
        assert_eq!(c.newer, ["2.33.46", "2.33.45"]);
        let r = c.latest.unwrap();
        assert_eq!((r.tag.as_str(), r.sha256.as_deref()), ("2.33.46", Some("abc")));
        assert!(!c.held);
        let held = serde_json::json!([
            release("2.34.0", "big change"),
            release("2.33.46", HELD_MARKER),
            release("2.33.45", "")
        ]);
        assert!(read_releases(&held.to_string(), "2.33.45").unwrap().held);
        // A hold on an older release doesn't matter.
        assert!(!read_releases(&held.to_string(), "2.33.46").unwrap().held);
        // A list that doesn't reach back to the installed release may hide a hold.
        let c = read_releases(&list.to_string(), "2.33.40").unwrap();
        assert!(c.held && c.latest.is_some());
        assert!(read_releases(&list.to_string(), "2.33.46").unwrap().latest.is_none());
        let mut pre = release("2.40.0", "");
        pre["prerelease"] = serde_json::json!(true);
        assert!(read_releases(&serde_json::json!([pre]).to_string(), "2.33.46").unwrap().latest.is_none());
    }

    fn zip_of(path: &Path, files: &[(&str, &str)]) {
        let mut w = zip::ZipWriter::new(std::fs::File::create(path).unwrap());
        for (name, text) in files {
            w.start_file(*name, zip::write::SimpleFileOptions::default()).unwrap();
            w.write_all(text.as_bytes()).unwrap();
        }
        w.finish().unwrap();
    }

    #[test]
    fn an_update_keeps_what_smf_keeps_and_backs_up_what_it_replaces() {
        let tmp = tempfile::tempdir().unwrap();
        let smf = tmp.path().join("smf");
        std::fs::create_dir_all(smf.join("Mods/M")).unwrap();
        std::fs::create_dir_all(smf.join("Load Order Manager")).unwrap();
        std::fs::write(smf.join("Deploy.exe"), "old").unwrap();
        std::fs::write(smf.join("config.json"), "mine").unwrap();
        std::fs::write(smf.join("Mods/M/manifest.json"), "mod").unwrap();
        std::fs::write(smf.join("Load Order Manager/x.exe"), "lom").unwrap();
        let zip = tmp.path().join("Release.zip");
        zip_of(
            &zip,
            &[
                ("Deploy.exe", "new"),
                ("config.json", "default"),
                ("Mods/Example/manifest.json", "example"),
                ("Third-Party/7z.exe", "7z"),
                ("Mod Manager/Mod Manager.exe", "electron"),
            ],
        );
        let staging = tmp.path().join("staging");
        extract(&zip, &staging).unwrap();
        prune(&staging).unwrap();
        let backup = tmp.path().join("backup");
        assert_eq!(apply(&staging, &smf, &backup).unwrap(), 2);

        let read = |p: &Path| std::fs::read_to_string(p).unwrap();
        assert_eq!(read(&smf.join("Deploy.exe")), "new");
        assert_eq!(read(&smf.join("Third-Party/7z.exe")), "7z");
        assert_eq!(read(&smf.join("config.json")), "mine");
        assert_eq!(read(&smf.join("Mods/M/manifest.json")), "mod");
        assert!(!smf.join("Mods/Example").exists() && !smf.join("Mod Manager").exists());
        assert!(!smf.join("Load Order Manager").exists());
        assert_eq!(read(&backup.join("Deploy.exe")), "old");
        assert_eq!(read(&backup.join("Load Order Manager/x.exe")), "lom");
    }

    #[test]
    fn a_zip_that_escapes_its_folder_is_refused() {
        let tmp = tempfile::tempdir().unwrap();
        let zip = tmp.path().join("evil.zip");
        zip_of(&zip, &[("../outside.txt", "x")]);
        let err = extract(&zip, &tmp.path().join("staging")).unwrap_err();
        assert_eq!(err.code, ErrorCode::DownloadMismatch);
        assert!(!tmp.path().join("outside.txt").exists());
    }
}
