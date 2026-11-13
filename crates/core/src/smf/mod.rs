//! Simple Mod Framework, read through the files SMF documents for its users:
//! `config.json`, the `Mods` folder and each mod's `manifest.json`. The only
//! writes, to `config.json` (a mod on or off, a mod's options), live in
//! [`write`](mod@write). Nothing here runs SMF.

mod files;
mod kind;
pub mod options;
mod releases;
pub mod write;

use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use serde::Deserialize;
use sha2::{Digest, Sha256};

use crate::framework::{Framework, Inspection, ModEntry, ModKind, OrderRule, Relation, Status, Version};
use crate::game::GameInstall;

/// A file SMF itself keeps in `Mods`; it is not a mod.
const MANAGED_MARKER: &str = "Managed by SMF, do not touch";

/// Remembers the last version it worked out, so a refresh does not hash
/// `Deploy.exe` again unless its size or modified time changed, and each
/// mod's kind, so a refresh does not walk every mod's files again.
#[derive(Default)]
pub struct Smf {
    version_cache: RefCell<Option<(VersionKey, Version)>>,
    kinds: KindCache,
    /// Where a copy of `config.json` goes before each write. `None` keeps no
    /// copy, which only tests use.
    backup_dir: Option<PathBuf>,
}

type VersionKey = (PathBuf, u64, Option<SystemTime>);

impl Smf {
    pub fn with_backups(backup_dir: PathBuf) -> Self {
        Smf { backup_dir: Some(backup_dir), ..Smf::default() }
    }

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
        let (mods, notes) = list_mods(&folder.join("Mods"), &config, &self.kinds);
        Inspection { status: Status::Installed { folder, version }, mods, notes }
    }

    fn set_enabled(&self, game: &GameInstall, mod_id: &str, enabled: bool) -> Result<bool, String> {
        let folder = find_folder(&game.root).ok_or("Simple Mod Framework is no longer in the game folder.")?;
        let mut options = None;
        let mut mods = Vec::new();
        if enabled {
            let m = self.find_mod(&folder, mod_id)?;
            if let Some(problem) = &m.problem {
                return Err(format!("{} can't be turned on. {problem}", m.name));
            }
            let config = read_config(&folder)?;
            if m.badge.as_deref() != Some("RPKG") {
                let manifest = read_manifest(&m.folder.join("manifest.json"))?;
                if !manifest.options.is_empty() {
                    options = Some(options::default_values(&manifest.options, &config.load_order));
                }
            }
            mods = list_mods(&folder.join("Mods"), &config, &self.kinds).0;
        }
        let place = |ids: &[String]| crate::order::insert_position(ids, mod_id, &mods);
        let place: write::Place = if enabled { Some(&place) } else { None };
        write::set_enabled(&folder, mod_id, enabled, options.as_deref(), place, self.backup_dir.as_deref())
    }

    fn set_load_order(&self, game: &GameInstall, order: &[String]) -> Result<bool, String> {
        let folder = find_folder(&game.root).ok_or("Simple Mod Framework is no longer in the game folder.")?;
        write::set_load_order(&folder, order, self.backup_dir.as_deref())
    }

    fn set_options(&self, game: &GameInstall, mod_id: &str, chosen: &[String]) -> Result<bool, String> {
        let folder = find_folder(&game.root).ok_or("Simple Mod Framework is no longer in the game folder.")?;
        let m = self.find_mod(&folder, mod_id)?;
        if let Some(problem) = &m.problem {
            return Err(format!("{}'s options can't be changed. {problem}", m.name));
        }
        if m.badge.as_deref() == Some("RPKG") {
            return Err(format!("{} is an RPKG mod, which has no options.", m.name));
        }
        let manifest = read_manifest(&m.folder.join("manifest.json"))?;
        let config = read_config(&folder)?;
        let values = options::validate(&manifest.options, chosen, &config.load_order)?;
        write::set_options(&folder, mod_id, &values, self.backup_dir.as_deref())
    }
}

impl Smf {
    /// The mod in SMF's Mods folder with this id.
    fn find_mod(&self, folder: &Path, mod_id: &str) -> Result<ModEntry, String> {
        let (mods, _) = list_mods(&folder.join("Mods"), &Config::default(), &self.kinds);
        mods.into_iter()
            .find(|m| m.id == mod_id)
            .ok_or_else(|| format!("\"{mod_id}\" is no longer in SMF's Mods folder."))
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
    /// Each mod's chosen options. Read loosely: an entry that is not a list
    /// of strings counts as no entry.
    #[serde(rename = "modOptions", default)]
    pub mod_options: BTreeMap<String, serde_json::Value>,
}

impl Config {
    fn options_of(&self, mod_id: &str) -> Option<Vec<String>> {
        let list = self.mod_options.get(mod_id)?.as_array()?;
        list.iter().map(|v| v.as_str().map(str::to_string)).collect()
    }
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
    #[serde(default)]
    options: Vec<serde_json::Value>,
    #[serde(default)]
    localisation: Option<serde::de::IgnoredAny>,
    #[serde(rename = "localisationOverrides", default)]
    localisation_overrides: Option<serde::de::IgnoredAny>,
    #[serde(rename = "localisedLines", default)]
    localised_lines: Option<serde::de::IgnoredAny>,
    #[serde(rename = "contentFolders", default)]
    content_folders: Vec<serde_json::Value>,
    #[serde(rename = "loadBefore", default)]
    load_before: Vec<serde_json::Value>,
    #[serde(rename = "loadAfter", default)]
    load_after: Vec<serde_json::Value>,
}

/// One `loadBefore`/`loadAfter` entry before the other mod's version is known:
/// an id, or `[id, version range]`.
struct RawRule {
    relation: Relation,
    other: String,
    range: Option<String>,
}

fn raw_rules(relation: Relation, entries: &[serde_json::Value]) -> impl Iterator<Item = RawRule> + '_ {
    entries.iter().filter_map(move |e| match e {
        serde_json::Value::String(id) => Some(RawRule { relation, other: id.clone(), range: None }),
        serde_json::Value::Array(pair) => Some(RawRule {
            relation,
            other: pair.first()?.as_str()?.to_string(),
            range: Some(pair.get(1)?.as_str()?.to_string()),
        }),
        _ => None,
    })
}

/// Whether a version range in a rule covers `version`. SMF uses npm's semver,
/// where a version or range that can't be read never matches; so here.
fn range_matches(range: &str, version: Option<&str>) -> bool {
    let (Ok(req), Some(Ok(v))) = (semver::VersionReq::parse(range), version.map(semver::Version::parse)) else {
        return false;
    };
    req.matches(&v)
}

impl Manifest {
    fn has_localisation(&self) -> bool {
        self.localisation.is_some() || self.localisation_overrides.is_some() || self.localised_lines.is_some()
    }
}

fn read_manifest(path: &Path) -> Result<Manifest, String> {
    std::fs::read_to_string(path)
        .map_err(|e| e.to_string())
        .and_then(|t| json5::from_str::<Manifest>(t.trim_start_matches('\u{feff}')).map_err(|e| e.to_string()))
}

/// Each mod's kind, and the files each content folder replaces, worked out
/// again only when the folder or its manifest changes. Walking a large mod's
/// files takes a moment.
#[derive(Default)]
pub struct KindCache {
    kinds: RefCell<HashMap<PathBuf, (KindKey, ModKind)>>,
    files: RefCell<HashMap<PathBuf, (KindKey, Vec<String>)>>,
}

type KindKey = (Option<SystemTime>, Option<SystemTime>, bool);

fn modified(p: &Path) -> Option<SystemTime> {
    std::fs::metadata(p).and_then(|m| m.modified()).ok()
}

impl KindCache {
    fn get(&self, folder: &Path, has_localisation: bool) -> ModKind {
        let key = (modified(folder), modified(&folder.join("manifest.json")), has_localisation);
        if let Some((cached, kind)) = self.kinds.borrow().get(folder)
            && *cached == key
        {
            return *kind;
        }
        let kind = kind::classify(folder, has_localisation);
        self.kinds.borrow_mut().insert(folder.to_path_buf(), (key, kind));
        kind
    }

    fn replaced_files(&self, mod_folder: &Path, content: &Path) -> Vec<String> {
        let key = (modified(content), modified(&mod_folder.join("manifest.json")), false);
        if let Some((cached, list)) = self.files.borrow().get(content)
            && *cached == key
        {
            return list.clone();
        }
        let list = files::raw_resources(content);
        self.files.borrow_mut().insert(content.to_path_buf(), (key, list.clone()));
        list
    }
}

/// Every mod folder in `Mods`, in load order first, then the mods that are
/// off, by name. A folder without `manifest.json` is an RPKG-only mod, which
/// SMF identifies by its folder name.
pub fn list_mods(mods_dir: &Path, config: &Config, kinds: &KindCache) -> (Vec<ModEntry>, Vec<String>) {
    let mut notes = Vec::new();
    let Ok(entries) = std::fs::read_dir(mods_dir) else {
        notes.push(format!("SMF's Mods folder is missing: {}", mods_dir.display()));
        return (Vec::new(), notes);
    };
    let mut mods = Vec::new();
    let mut pending: Vec<(String, Vec<RawRule>)> = Vec::new();
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
        mods.push(read_mod(&path, &file_name, kinds, config, &mut pending));
    }
    // Rules can name a version range of the other mod, so they are resolved
    // once every mod's version is known.
    for (id, raw) in pending {
        let rules: Vec<OrderRule> = raw
            .into_iter()
            .filter(|r| match &r.range {
                None => true,
                Some(range) => {
                    range_matches(range, mods.iter().find(|m| m.id == r.other).and_then(|m| m.version.as_deref()))
                }
            })
            .map(|r| OrderRule { relation: r.relation, other: r.other })
            .collect();
        if let Some(m) = mods.iter_mut().find(|m| m.id == id) {
            m.rules = rules;
        }
    }
    for m in &mut mods {
        m.load_position = config.load_order.iter().position(|id| *id == m.id).map(|i| i + 1);
    }
    // An id in the load order that no folder answers to: a mod that was
    // removed, or one whose manifest broke so its real id can't be read. It
    // gets a row so it can still be turned off.
    for (i, id) in config.load_order.iter().enumerate() {
        if mods.iter().any(|m| m.id == *id) || config.load_order[..i].contains(id) {
            continue;
        }
        mods.push(ModEntry {
            id: id.clone(),
            name: id.clone(),
            version: None,
            authors: Vec::new(),
            folder: mods_dir.join(id),
            badge: None,
            kind: ModKind::Other,
            load_position: Some(i + 1),
            options: Vec::new(),
            rules: Vec::new(),
            replaces: Vec::new(),
            problem: Some(
                "SMF's load order lists this id, but no mod in the Mods folder has it. Turn it off to take it out \
                 of the load order."
                    .into(),
            ),
        });
    }
    mods.sort_by(|a, b| match (a.load_position, b.load_position) {
        (Some(x), Some(y)) => x.cmp(&y),
        (Some(_), None) => std::cmp::Ordering::Less,
        (None, Some(_)) => std::cmp::Ordering::Greater,
        (None, None) => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
    });
    (mods, notes)
}

fn read_mod(
    path: &Path,
    folder_name: &str,
    kinds: &KindCache,
    config: &Config,
    pending: &mut Vec<(String, Vec<RawRule>)>,
) -> ModEntry {
    let base = ModEntry {
        id: folder_name.to_string(),
        name: folder_name.to_string(),
        version: None,
        authors: Vec::new(),
        folder: path.to_path_buf(),
        badge: None,
        kind: ModKind::Other,
        load_position: None,
        problem: None,
        options: Vec::new(),
        rules: Vec::new(),
        replaces: Vec::new(),
    };
    let manifest_path = path.join("manifest.json");
    if !manifest_path.is_file() {
        // RPKG-only mods are opaque packages, so their kind is not guessed.
        return ModEntry { badge: Some("RPKG".into()), ..base };
    }
    match read_manifest(&manifest_path) {
        Ok(m) => {
            let shown = options::describe(&m.options, config.options_of(&m.id).as_deref(), &config.load_order);
            // The chosen options' own content folders and load rules count too.
            let chosen: Vec<&serde_json::Value> = m
                .options
                .iter()
                .filter(|o| options::value_of(o).is_some_and(|v| shown.iter().any(|s| s.chosen && s.value == v)))
                .collect();
            let mut rules: Vec<RawRule> =
                raw_rules(Relation::Before, &m.load_before).chain(raw_rules(Relation::After, &m.load_after)).collect();
            let mut folders: Vec<String> =
                m.content_folders.iter().filter_map(|f| f.as_str().map(str::to_string)).collect();
            for o in &chosen {
                let list = |key: &str| o.get(key).and_then(|v| v.as_array()).cloned().unwrap_or_default();
                rules.extend(raw_rules(Relation::Before, &list("loadBefore")).collect::<Vec<_>>());
                rules.extend(raw_rules(Relation::After, &list("loadAfter")).collect::<Vec<_>>());
                folders.extend(list("contentFolders").iter().filter_map(|f| f.as_str().map(str::to_string)));
            }
            let mut replaces: Vec<String> = folders
                .iter()
                // A content folder must stay inside the mod's own folder.
                .filter(|f| !f.contains("..") && !Path::new(f).is_absolute())
                .flat_map(|f| kinds.replaced_files(path, &path.join(f)))
                .collect();
            replaces.sort();
            replaces.dedup();
            pending.push((m.id.clone(), rules));
            ModEntry {
                options: shown,
                kind: kinds.get(path, m.has_localisation()),
                replaces,
                id: m.id,
                name: m.name,
                version: m.version,
                authors: m.authors,
                ..base
            }
        }
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
    fn turns_mods_on_and_off_through_config_json() {
        let tmp = tempfile::tempdir().unwrap();
        let smf_folder = fake_install(tmp.path(), b"deploy");
        std::fs::write(
            smf_folder.join("Mods/Accent Overhaul/manifest.json"),
            "{ id: 'Someone.Accents', name: 'Accent Overhaul', options: [\
             { name: 'Japanese', type: 'checkbox', enabledByDefault: true },\
             { name: 'All', group: 'Region', type: 'select', enabledByDefault: true }] }",
        )
        .unwrap();
        let game = GameInstall { store: Store::Folder, root: tmp.path().to_path_buf() };
        let backups = tmp.path().join("backups");
        let smf = Smf::with_backups(backups.clone());

        assert!(smf.set_enabled(&game, "Someone.Accents", true).unwrap());
        assert!(!smf.set_enabled(&game, "Someone.Accents", true).unwrap());
        assert!(smf.set_enabled(&game, "Winter Suit", false).unwrap());
        let config: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(smf_folder.join("config.json")).unwrap()).unwrap();
        assert_eq!(config["loadOrder"], serde_json::json!(["Atampy26.RealisticAI", "Someone.Accents"]));
        assert_eq!(config["modOptions"]["Someone.Accents"], serde_json::json!(["Japanese", "Region:All"]));
        assert_eq!(config["skipIntro"], true);
        assert_eq!(std::fs::read_dir(&backups).unwrap().count(), 2);

        let order: Vec<Option<usize>> =
            smf.inspect(&game).mods.iter().filter(|m| m.name != "Broken").map(|m| m.load_position).collect();
        assert_eq!(order, [Some(1), Some(2), None]);

        assert!(smf.set_enabled(&game, "Broken", true).unwrap_err().contains("can't be turned on"));

        // An enabled mod whose manifest breaks keeps a row under its old id,
        // so it can still be turned off.
        std::fs::write(smf_folder.join("Mods/Accent Overhaul/manifest.json"), "{ broken").unwrap();
        let mods = smf.inspect(&game).mods;
        let orphan = mods.iter().find(|m| m.id == "Someone.Accents").expect("a row for the old id");
        assert_eq!(orphan.load_position, Some(2));
        assert!(orphan.problem.as_deref().unwrap().contains("no mod in the Mods folder"));
        assert!(smf.set_enabled(&game, "Someone.Accents", false).unwrap());
        assert!(smf.set_enabled(&game, "Nobody.Gone", true).unwrap_err().contains("no longer"));
        // A mod whose folder is gone can still be turned off.
        assert!(smf.set_enabled(&game, "Atampy26.RealisticAI", false).unwrap());
    }

    #[test]
    fn shows_and_saves_a_mods_options() {
        let tmp = tempfile::tempdir().unwrap();
        let smf_folder = fake_install(tmp.path(), b"deploy");
        std::fs::write(
            smf_folder.join("Mods/Realistic AI/manifest.json"),
            "{ id: 'Atampy26.RealisticAI', name: 'Realistic AI', options: [\
             { name: 'Harder', type: 'checkbox', enabledByDefault: true },\
             { name: 'Calm', group: 'Mood', type: 'select', enabledByDefault: true },\
             { name: 'Angry', group: 'Mood', type: 'select' },\
             { name: 'With accents', type: 'checkbox', requirements: ['Someone.Accents'] }] }",
        )
        .unwrap();
        let game = GameInstall { store: Store::Folder, root: tmp.path().to_path_buf() };
        let smf = Smf::default();
        let ai = |smf: &Smf| smf.inspect(&game).mods.into_iter().find(|m| m.id == "Atampy26.RealisticAI").unwrap();

        // On, with no modOptions entry yet: the defaults are shown.
        let chosen: Vec<String> = ai(&smf).options.into_iter().filter(|o| o.chosen).map(|o| o.value).collect();
        assert_eq!(chosen, ["Harder", "Mood:Calm"]);
        assert_eq!(ai(&smf).options[3].needs, ["Someone.Accents"]);

        assert!(smf.set_options(&game, "Atampy26.RealisticAI", &["Mood:Angry".into()]).unwrap());
        let chosen: Vec<String> = ai(&smf).options.into_iter().filter(|o| o.chosen).map(|o| o.value).collect();
        assert_eq!(chosen, ["Mood:Angry"]);

        let err = smf.set_options(&game, "Atampy26.RealisticAI", &["With accents".into()]).unwrap_err();
        assert!(err.contains("needs Someone.Accents"), "{err}");
        assert!(smf.set_enabled(&game, "Someone.Accents", true).unwrap());
        assert!(smf.set_options(&game, "Atampy26.RealisticAI", &["With accents".into()]).unwrap());

        assert!(smf.set_options(&game, "Winter Suit", &[]).unwrap_err().contains("RPKG"));
        assert!(smf.set_options(&game, "Broken", &[]).unwrap_err().contains("can't be changed"));
    }

    #[test]
    fn reads_load_rules_and_replaced_files() {
        let tmp = tempfile::tempdir().unwrap();
        let smf_folder = fake_install(tmp.path(), b"deploy");
        std::fs::write(
            smf_folder.join("Mods/Realistic AI/manifest.json"),
            "{ id: 'Atampy26.RealisticAI', name: 'Realistic AI', version: '1.2.0', contentFolders: ['content'],\
             loadBefore: ['Winter Suit', ['Someone.Accents', '>=2.0.0']], loadAfter: [['Someone.Accents', '^1']],\
             options: [{ name: 'Extra', type: 'checkbox', enabledByDefault: true, contentFolders: ['extra'],\
                         loadAfter: ['Other.Mod'] }] }",
        )
        .unwrap();
        std::fs::write(
            smf_folder.join("Mods/Accent Overhaul/manifest.json"),
            "{ id: 'Someone.Accents', name: 'Accent Overhaul', version: '1.4.0', contentFolders: ['c', '../escape'] }",
        )
        .unwrap();
        for f in [
            "Realistic AI/content/chunk0/00AA00AA00AA00AA.TEMP",
            "Realistic AI/content/chunk0/00AA00AA00AA00AA.TEMP.meta",
            "Realistic AI/extra/chunk1/00BB00BB00BB00BB.PRIM",
            "Realistic AI/unused/00CC00CC00CC00CC.PRIM",
            "Accent Overhaul/c/chunk0/00aa00aa00aa00aa.temp",
        ] {
            let p = smf_folder.join("Mods").join(f);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, b"").unwrap();
        }
        let game = GameInstall { store: Store::Folder, root: tmp.path().to_path_buf() };
        let mods = Smf::default().inspect(&game).mods;
        let ai = mods.iter().find(|m| m.id == "Atampy26.RealisticAI").unwrap();
        let rules: Vec<(Relation, &str)> = ai.rules.iter().map(|r| (r.relation, r.other.as_str())).collect();
        // The >=2.0.0 rule doesn't cover Accents 1.4.0; ^1 does.
        assert_eq!(
            rules,
            [(Relation::Before, "Winter Suit"), (Relation::After, "Someone.Accents"), (Relation::After, "Other.Mod")]
        );
        assert_eq!(ai.replaces, ["00AA00AA00AA00AA.TEMP", "00BB00BB00BB00BB.PRIM"]);
        let accents = mods.iter().find(|m| m.id == "Someone.Accents").unwrap();
        assert_eq!(accents.replaces, ["00AA00AA00AA00AA.TEMP"]);
    }

    #[test]
    fn a_turned_on_mod_lands_where_its_rules_allow_and_the_order_can_be_saved() {
        let tmp = tempfile::tempdir().unwrap();
        let smf_folder = fake_install(tmp.path(), b"deploy");
        std::fs::write(
            smf_folder.join("Mods/Accent Overhaul/manifest.json"),
            "{ id: 'Someone.Accents', name: 'Accent Overhaul', loadBefore: ['Atampy26.RealisticAI'] }",
        )
        .unwrap();
        let game = GameInstall { store: Store::Folder, root: tmp.path().to_path_buf() };
        let smf = Smf::default();
        assert!(smf.set_enabled(&game, "Someone.Accents", true).unwrap());
        let order = || read_config(&smf_folder).unwrap().load_order;
        assert_eq!(order(), ["Winter Suit", "Someone.Accents", "Atampy26.RealisticAI"]);

        let reordered: Vec<String> =
            ["Atampy26.RealisticAI", "Winter Suit", "Someone.Accents"].map(String::from).to_vec();
        assert!(smf.set_load_order(&game, &reordered).unwrap());
        assert_eq!(order(), reordered);
        assert!(smf.set_load_order(&game, &reordered[..2]).is_err());
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
        assert_eq!(
            releases::version_for("51F94580995879EC189F4E6C129BD856ED34D86CC94B386F0A48087DAB0B11E4"),
            Some("2.33.45")
        );
        assert_eq!(releases::version_for("00"), None);
    }
}
