//! Simple Mod Framework, read through the files SMF documents for its users:
//! `config.json`, the `Mods` folder and each mod's `manifest.json`. Nothing
//! here writes, and nothing here runs SMF.

mod releases;

use std::cell::RefCell;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use serde::Deserialize;
use sha2::{Digest, Sha256};

use crate::framework::{Framework, Inspection, ModEntry, Status, Version};
use crate::game::GameInstall;

/// A file SMF itself keeps in `Mods`; it is not a mod.
const MANAGED_MARKER: &str = "Managed by SMF, do not touch";

/// Remembers the last version it worked out, so a refresh does not hash
/// `Deploy.exe` again unless its size or modified time changed.
#[derive(Default)]
pub struct Smf {
    version_cache: RefCell<Option<(VersionKey, Version)>>,
}

type VersionKey = (PathBuf, u64, Option<SystemTime>);

impl Smf {
    fn version(&self, deploy_exe: &Path) -> Result<Version, String> {
        let meta =
            std::fs::metadata(deploy_exe).map_err(|e| format!("Could not read {}: {e}", deploy_exe.display()))?;
        let key: VersionKey = (deploy_exe.to_path_buf(), meta.len(), meta.modified().ok());
        if let Some((cached_key, version)) = &*self.version_cache.borrow()
            && *cached_key == key
        {
            return Ok(version.clone());
        }
        let version = detect_version(deploy_exe)?;
        *self.version_cache.borrow_mut() = Some((key, version.clone()));
        Ok(version)
    }
}

impl Framework for Smf {
    fn name(&self) -> &'static str {
        "Simple Mod Framework"
    }

    fn inspect(&self, game: &GameInstall) -> Inspection {
        let Some(folder) = find_folder(&game.root) else {
            return Inspection { status: Status::NotInstalled, mods: Vec::new(), notes: Vec::new() };
        };
        let config = match read_config(&folder) {
            Ok(c) => c,
            Err(reason) => {
                return Inspection {
                    status: Status::Unreadable { folder, reason },
                    mods: Vec::new(),
                    notes: Vec::new(),
                };
            }
        };
        let version = match self.version(&folder.join("Deploy.exe")) {
            Ok(v) => v,
            Err(reason) => {
                return Inspection {
                    status: Status::Unreadable { folder, reason },
                    mods: Vec::new(),
                    notes: Vec::new(),
                };
            }
        };
        let (mods, notes) = list_mods(&folder.join("Mods"), &config);
        Inspection { status: Status::Installed { folder, version }, mods, notes }
    }
}

/// SMF lives in any folder directly inside the game root (its default
/// `retailPath` is `..\Retail`). The folder named "Simple Mod Framework" wins
/// when there is more than one.
pub fn find_folder(game_root: &Path) -> Option<PathBuf> {
    let mut candidates: Vec<PathBuf> = std::fs::read_dir(game_root)
        .ok()?
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.join("Deploy.exe").is_file() && p.join("config.json").is_file())
        .collect();
    candidates.sort_by_key(|p| !crate::paths::file_name_eq(p, "Simple Mod Framework"));
    candidates.into_iter().next()
}

/// The parts of SMF's `config.json` this app reads. Other keys are ignored.
#[derive(Debug, Default, Deserialize)]
pub struct Config {
    #[serde(rename = "loadOrder", default)]
    pub load_order: Vec<String>,
}

pub fn read_config(folder: &Path) -> Result<Config, String> {
    let path = folder.join("config.json");
    let text = std::fs::read_to_string(&path).map_err(|e| format!("Could not read {}: {e}", path.display()))?;
    json5::from_str(text.trim_start_matches('\u{feff}'))
        .map_err(|e| format!("SMF's config.json is not valid JSON ({e}). Fix or reset it in SMF before continuing."))
}

/// Identifies the installed release by `Deploy.exe`'s SHA-256. Releases do
/// not record their version anywhere else that SMF ships.
pub fn detect_version(deploy_exe: &Path) -> Result<Version, String> {
    let sha256 = sha256_file(deploy_exe).map_err(|e| format!("Could not read {}: {e}", deploy_exe.display()))?;
    Ok(match releases::version_for(&sha256) {
        Some(v) => Version::Known(v.to_string()),
        None => Version::Unknown { sha256 },
    })
}

fn sha256_file(path: &Path) -> std::io::Result<String> {
    let mut file = std::fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 1 << 16];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hasher.finalize().iter().map(|b| format!("{b:02x}")).collect())
}

#[derive(Debug, Deserialize)]
struct Manifest {
    id: String,
    name: String,
    #[serde(default)]
    version: Option<String>,
    #[serde(default)]
    authors: Vec<String>,
}

/// Every mod folder in `Mods`, in load order first, then the mods that are
/// off, by name. A folder without `manifest.json` is an RPKG-only mod, which
/// SMF identifies by its folder name.
pub fn list_mods(mods_dir: &Path, config: &Config) -> (Vec<ModEntry>, Vec<String>) {
    let mut notes = Vec::new();
    let Ok(entries) = std::fs::read_dir(mods_dir) else {
        notes.push(format!("SMF's Mods folder is missing: {}", mods_dir.display()));
        return (Vec::new(), notes);
    };
    let mut mods = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        let file_name = entry.file_name().to_string_lossy().into_owned();
        if !path.is_dir() {
            if file_name != MANAGED_MARKER {
                notes.push(format!(
                    "\"{file_name}\" is a file in SMF's Mods folder. SMF only reads folders there, so it is ignored."
                ));
            }
            continue;
        }
        mods.push(read_mod(&path, &file_name));
    }
    for m in &mut mods {
        m.load_position = config.load_order.iter().position(|id| *id == m.id).map(|i| i + 1);
    }
    mods.sort_by(|a, b| match (a.load_position, b.load_position) {
        (Some(x), Some(y)) => x.cmp(&y),
        (Some(_), None) => std::cmp::Ordering::Less,
        (None, Some(_)) => std::cmp::Ordering::Greater,
        (None, None) => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
    });
    (mods, notes)
}

fn read_mod(path: &Path, folder_name: &str) -> ModEntry {
    let base = ModEntry {
        id: folder_name.to_string(),
        name: folder_name.to_string(),
        version: None,
        authors: Vec::new(),
        folder: path.to_path_buf(),
        badge: None,
        load_position: None,
        problem: None,
    };
    let manifest_path = path.join("manifest.json");
    if !manifest_path.is_file() {
        return ModEntry { badge: Some("RPKG".into()), ..base };
    }
    let parsed = std::fs::read_to_string(&manifest_path)
        .map_err(|e| e.to_string())
        .and_then(|t| json5::from_str::<Manifest>(t.trim_start_matches('\u{feff}')).map_err(|e| e.to_string()));
    match parsed {
        Ok(m) => ModEntry { id: m.id, name: m.name, version: m.version, authors: m.authors, ..base },
        Err(e) => ModEntry { problem: Some(format!("Its manifest.json could not be read: {e}")), ..base },
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;
    use crate::game::Store;

    fn fake_install(root: &Path, deploy: &[u8]) -> PathBuf {
        let smf = root.join("Simple Mod Framework");
        std::fs::create_dir_all(smf.join("Mods/Realistic AI")).unwrap();
        std::fs::create_dir_all(smf.join("Mods/Winter Suit")).unwrap();
        std::fs::create_dir_all(smf.join("Mods/Broken")).unwrap();
        std::fs::create_dir_all(smf.join("Mods/Accent Overhaul")).unwrap();
        std::fs::write(smf.join("Deploy.exe"), deploy).unwrap();
        std::fs::write(
            smf.join("config.json"),
            "{\n  // comments are allowed\n  \"loadOrder\": [\"Winter Suit\", \"Atampy26.RealisticAI\"],\n  \"skipIntro\": true,\n}\n",
        )
        .unwrap();
        std::fs::write(smf.join(format!("Mods/{MANAGED_MARKER}")), b"").unwrap();
        std::fs::write(smf.join("Mods/stray.zip"), b"zip").unwrap();
        std::fs::write(
            smf.join("Mods/Realistic AI/manifest.json"),
            "{\n\t\"$schema\": \"x\",\n\t\"id\": \"Atampy26.RealisticAI\", // the id\n\t\"name\": \"Realistic AI\",\n\t\"version\": \"1.2.0\",\n\t\"authors\": [\"Atampy26\"],\n\t\"frameworkVersion\": \"2.33.42\",\n}\n",
        )
        .unwrap();
        std::fs::write(
            smf.join("Mods/Accent Overhaul/manifest.json"),
            "{ \"id\": \"Someone.Accents\", \"name\": \"Accent Overhaul\", \"authors\": [] }",
        )
        .unwrap();
        std::fs::write(smf.join("Mods/Broken/manifest.json"), "{ \"id\": ").unwrap();
        smf
    }

    /// Every entry, with each file's bytes and modified time. Folder times
    /// are left out: NTFS can update a folder's time a moment after a file
    /// is created in it, which made this test flaky. Any file created,
    /// deleted or renamed still shows up as a changed set of entries.
    fn snapshot(dir: &Path) -> BTreeMap<PathBuf, Option<(Vec<u8>, std::time::SystemTime)>> {
        let mut out = BTreeMap::new();
        let mut stack = vec![dir.to_path_buf()];
        while let Some(d) = stack.pop() {
            for e in std::fs::read_dir(&d).unwrap().flatten() {
                let p = e.path();
                let meta = e.metadata().unwrap();
                if meta.is_dir() {
                    stack.push(p.clone());
                    out.insert(p, None);
                } else {
                    out.insert(p.clone(), Some((std::fs::read(&p).unwrap(), meta.modified().unwrap())));
                }
            }
        }
        out
    }

    #[test]
    fn inspects_an_install_and_its_mods() {
        let tmp = tempfile::tempdir().unwrap();
        let smf = fake_install(tmp.path(), b"not the real Deploy.exe");
        let game = GameInstall { store: Store::Folder, root: tmp.path().to_path_buf() };
        let inspection = Smf::default().inspect(&game);

        let Status::Installed { folder, version } = &inspection.status else { panic!("{:?}", inspection.status) };
        assert_eq!(folder, &smf);
        assert!(matches!(version, Version::Unknown { sha256 } if sha256.len() == 64));

        let names: Vec<&str> = inspection.mods.iter().map(|m| m.name.as_str()).collect();
        assert_eq!(names, ["Winter Suit", "Realistic AI", "Accent Overhaul", "Broken"]);
        let positions: Vec<Option<usize>> = inspection.mods.iter().map(|m| m.load_position).collect();
        assert_eq!(positions, [Some(1), Some(2), None, None]);

        let winter = &inspection.mods[0];
        assert_eq!(winter.badge.as_deref(), Some("RPKG"));
        let ai = &inspection.mods[1];
        assert_eq!((ai.id.as_str(), ai.version.as_deref()), ("Atampy26.RealisticAI", Some("1.2.0")));
        assert_eq!(ai.authors, ["Atampy26"]);
        assert!(inspection.mods[3].problem.is_some());

        assert_eq!(inspection.notes.len(), 1, "{:?}", inspection.notes);
        assert!(inspection.notes[0].contains("stray.zip"));
    }

    #[test]
    fn inspecting_never_modifies_the_install() {
        let tmp = tempfile::tempdir().unwrap();
        fake_install(tmp.path(), b"deploy");
        let before = snapshot(tmp.path());
        let game = GameInstall { store: Store::Folder, root: tmp.path().to_path_buf() };
        let smf = Smf::default();
        let _ = smf.inspect(&game);
        let _ = smf.inspect(&game);
        let after = snapshot(tmp.path());
        let changed: Vec<&PathBuf> =
            before.keys().chain(after.keys()).filter(|p| before.get(*p) != after.get(*p)).collect();
        assert!(changed.is_empty(), "inspection changed {changed:?}");
    }

    #[test]
    fn rehashes_only_when_deploy_exe_changes() {
        let tmp = tempfile::tempdir().unwrap();
        let folder = fake_install(tmp.path(), b"first");
        let game = GameInstall { store: Store::Folder, root: tmp.path().to_path_buf() };
        let smf = Smf::default();
        let version = |i: Inspection| match i.status {
            Status::Installed { version, .. } => version,
            other => panic!("{other:?}"),
        };
        let first = version(smf.inspect(&game));
        assert_eq!(first, version(smf.inspect(&game)));
        std::fs::write(folder.join("Deploy.exe"), b"a different, longer exe").unwrap();
        assert_ne!(first, version(smf.inspect(&game)));
    }

    #[test]
    fn reports_a_missing_or_unreadable_framework() {
        let tmp = tempfile::tempdir().unwrap();
        let game = GameInstall { store: Store::Folder, root: tmp.path().to_path_buf() };
        assert_eq!(Smf::default().inspect(&game).status, Status::NotInstalled);

        let smf = fake_install(tmp.path(), b"deploy");
        std::fs::write(smf.join("config.json"), "{ not json").unwrap();
        assert!(matches!(Smf::default().inspect(&game).status, Status::Unreadable { .. }));
    }

    #[test]
    fn prefers_the_folder_named_simple_mod_framework() {
        let tmp = tempfile::tempdir().unwrap();
        for name in ["Another Copy", "Simple Mod Framework"] {
            let dir = tmp.path().join(name);
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(dir.join("Deploy.exe"), b"").unwrap();
            std::fs::write(dir.join("config.json"), b"{}").unwrap();
        }
        assert!(crate::paths::file_name_eq(&find_folder(tmp.path()).unwrap(), "Simple Mod Framework"));
    }

    #[test]
    fn known_hash_maps_to_its_release() {
        assert_eq!(
            releases::version_for("f24f472f0d071c90f293bb67e49b9834d5c01188b3c2d278a3e04af86cd6907b"),
            Some("2.33.42")
        );
        assert_eq!(releases::version_for("00"), None);
    }
}
