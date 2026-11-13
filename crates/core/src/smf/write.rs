//! The only code in the core that writes: turning a mod on or off in SMF's
//! `config.json`, the way SMF's own Mod Manager does (`modList/+page.svelte`
//! and `getConfig` in `lib/utils.ts`, SMF 2.33.45).
//!
//! - On appends the mod's id to `loadOrder`. A mod with options and no entry
//!   in `modOptions` gets the options its author marked `enabledByDefault`,
//!   which SMF's Mod Manager fills in on its next read. `Deploy.exe` applies
//!   no options at all for a mod without an entry.
//! - Off removes the id from `loadOrder` and leaves its options, as SMF does.
//! - Every other key, and the order of keys, is kept as it was.
//!
//! The previous file is copied to the backup folder before each write, and
//! the new one replaces it in one rename, so SMF never reads half a file.
//! Just before the rename the file is read again: if anything else (SMF's
//! own Mod Manager, a second copy of this app) changed it in the meantime,
//! nothing is written, so neither change silently undoes the other.

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{Map, Value};

/// How many earlier copies of `config.json` the backup folder keeps.
pub const BACKUPS_KEPT: usize = 20;
/// The new file's name while it is written. Each process uses its own.
fn temp_name() -> String {
    format!("config.json.vmm-{}.tmp", std::process::id())
}

/// The options SMF selects when a mod is first turned on: checkbox and
/// select options marked `enabledByDefault`, stored as SMF stores them
/// (`name` for a checkbox, `group:name` for a select).
pub fn default_options(options: &[Value]) -> Vec<String> {
    options
        .iter()
        .filter(|o| o.get("enabledByDefault").and_then(Value::as_bool).unwrap_or(false))
        .filter_map(|o| {
            let name = o.get("name")?.as_str()?;
            match o.get("type")?.as_str()? {
                "checkbox" => Some(name.to_string()),
                "select" => Some(format!("{}:{name}", o.get("group")?.as_str()?)),
                _ => None,
            }
        })
        .collect()
}

/// Changes `config` in memory. `options` is `None` for a mod without options.
/// Returns whether anything changed.
pub fn apply(config: &mut Value, mod_id: &str, enabled: bool, options: Option<&[String]>) -> Result<bool, String> {
    let Some(root) = config.as_object_mut() else {
        return Err("SMF's config.json does not hold a settings object. Fix or reset it in SMF first.".into());
    };
    let order = root.entry("loadOrder").or_insert_with(|| Value::Array(Vec::new()));
    let Some(order) = order.as_array_mut() else {
        return Err("The load order in SMF's config.json is not a list. Fix or reset it in SMF first.".into());
    };
    let present = order.iter().any(|v| v.as_str() == Some(mod_id));
    if enabled == present {
        return Ok(false);
    }
    if !enabled {
        order.retain(|v| v.as_str() != Some(mod_id));
        return Ok(true);
    }
    order.push(Value::String(mod_id.to_string()));
    if let Some(options) = options {
        let all = root.entry("modOptions").or_insert_with(|| Value::Object(Map::new()));
        if let Some(all) = all.as_object_mut()
            && !all.contains_key(mod_id)
        {
            all.insert(mod_id.to_string(), Value::Array(options.iter().cloned().map(Value::String).collect()));
        }
    }
    Ok(true)
}

/// Reads `config.json`, applies the change and writes it back. The text is
/// JSON5 as SMF writes it; the result is plain JSON, which SMF reads the same.
pub fn set_enabled(
    smf_folder: &Path,
    mod_id: &str,
    enabled: bool,
    options: Option<&[String]>,
    backup_dir: Option<&Path>,
) -> Result<bool, String> {
    let path = smf_folder.join("config.json");
    let original = std::fs::read_to_string(&path).map_err(|e| format!("Could not read {}: {e}", path.display()))?;
    let mut config: Value = json5::from_str(original.trim_start_matches('\u{feff}'))
        .map_err(|e| format!("SMF's config.json is not valid JSON ({e}). Fix or reset it in SMF before continuing."))?;
    if !apply(&mut config, mod_id, enabled, options)? {
        return Ok(false);
    }
    let mut text = Vec::new();
    let formatter = serde_json::ser::PrettyFormatter::with_indent(b"    ");
    let mut ser = serde_json::Serializer::with_formatter(&mut text, formatter);
    serde::Serialize::serialize(&config, &mut ser).map_err(|e| format!("Could not write the new settings: {e}"))?;
    text.push(b'\n');

    if let Some(dir) = backup_dir {
        backup(dir, &original)?;
    }
    replace_if_unchanged(&path, &original, &text)?;
    Ok(true)
}

/// Writes `text` beside `path` and renames it over `path`, unless `path` no
/// longer holds `original`. The temporary file never outlives the call.
fn replace_if_unchanged(path: &Path, original: &str, text: &[u8]) -> Result<(), String> {
    let temp = path.with_file_name(temp_name());
    let result = std::fs::write(&temp, text)
        .map_err(|e| format!("Could not save SMF's config.json: {e}. Nothing was changed."))
        .and_then(|()| {
            if std::fs::read_to_string(path).ok().as_deref() == Some(original) {
                Ok(())
            } else {
                Err("Another program changed SMF's config.json while this change was being saved. \
                     Nothing was changed. Try again."
                    .to_string())
            }
        })
        .and_then(|()| {
            std::fs::rename(&temp, path)
                .map_err(|e| format!("Could not save SMF's config.json: {e}. Nothing was changed."))
        });
    if result.is_err() {
        let _ = std::fs::remove_file(&temp);
    }
    result
}

/// Saves `text` as `config-<milliseconds>.json` and keeps the newest
/// [`BACKUPS_KEPT`]. A failed backup stops the write.
fn backup(dir: &Path, text: &str) -> Result<PathBuf, String> {
    let fail = |e: std::io::Error| {
        format!("Could not keep a copy of SMF's config.json in {}: {e}. Nothing was changed.", dir.display())
    };
    std::fs::create_dir_all(dir).map_err(fail)?;
    let millis = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0);
    let mut file = dir.join(format!("config-{millis:015}.json"));
    let mut n = 1;
    while file.exists() {
        file = dir.join(format!("config-{millis:015}-{n}.json"));
        n += 1;
    }
    std::fs::write(&file, text).map_err(fail)?;
    let mut old: Vec<PathBuf> = std::fs::read_dir(dir)
        .map_err(fail)?
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.starts_with("config-") && n.ends_with(".json"))
        })
        .collect();
    old.sort();
    let excess = old.len().saturating_sub(BACKUPS_KEPT);
    for p in &old[..excess] {
        let _ = std::fs::remove_file(p);
    }
    Ok(file)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn folder_with(config: &str) -> tempfile::TempDir {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("config.json"), config).unwrap();
        tmp
    }

    fn read(dir: &Path) -> Value {
        serde_json::from_str(&std::fs::read_to_string(dir.join("config.json")).unwrap()).unwrap()
    }

    // The shape SMF's Mod Manager writes: json5.stringify, single quotes.
    const SMF_WRITTEN: &str = "{runtimePath:'..\\\\Runtime',retailPath:'..\\\\Retail',skipIntro:false,loadOrder:['A','B'],\
        modOptions:{'B':['Hands — Signature Suit:Bare hands']},knownMods:['A','B','C']}";

    #[test]
    fn turning_on_appends_and_keeps_everything_else() {
        let tmp = folder_with(SMF_WRITTEN);
        assert!(set_enabled(tmp.path(), "C", true, None, None).unwrap());
        let text = std::fs::read_to_string(tmp.path().join("config.json")).unwrap();
        let keys: Vec<&str> =
            ["runtimePath", "retailPath", "skipIntro", "loadOrder", "modOptions", "knownMods"].to_vec();
        let positions: Vec<usize> = keys.iter().map(|k| text.find(&format!("\"{k}\"")).unwrap()).collect();
        assert!(positions.windows(2).all(|w| w[0] < w[1]), "key order changed:\n{text}");
        let c = read(tmp.path());
        assert_eq!(c["loadOrder"], serde_json::json!(["A", "B", "C"]));
        assert_eq!(c["runtimePath"], "..\\Runtime");
        assert_eq!(c["modOptions"]["B"], serde_json::json!(["Hands — Signature Suit:Bare hands"]));
        assert_eq!(c["knownMods"], serde_json::json!(["A", "B", "C"]));
        assert!(!tmp.path().join(temp_name()).exists());
    }

    #[test]
    fn turning_off_removes_it_and_keeps_its_options() {
        let tmp = folder_with(SMF_WRITTEN);
        assert!(set_enabled(tmp.path(), "B", false, None, None).unwrap());
        let c = read(tmp.path());
        assert_eq!(c["loadOrder"], serde_json::json!(["A"]));
        assert_eq!(c["modOptions"]["B"], serde_json::json!(["Hands — Signature Suit:Bare hands"]));
    }

    #[test]
    fn no_change_writes_nothing() {
        let tmp = folder_with(SMF_WRITTEN);
        let backups = tmp.path().join("backups");
        assert!(!set_enabled(tmp.path(), "A", true, None, Some(&backups)).unwrap());
        assert!(!set_enabled(tmp.path(), "C", false, None, Some(&backups)).unwrap());
        assert_eq!(std::fs::read_to_string(tmp.path().join("config.json")).unwrap(), SMF_WRITTEN);
        assert!(!backups.exists());
    }

    #[test]
    fn first_time_on_selects_the_default_options() {
        let manifest_options = serde_json::json!([
            { "name": "Darker", "type": "checkbox", "enabledByDefault": true },
            { "name": "Brighter", "type": "checkbox" },
            { "name": "Bare hands", "group": "Hands", "type": "select", "enabledByDefault": true },
            { "name": "Gloves", "group": "Hands", "type": "select", "enabledByDefault": false },
            { "name": "Needs X", "type": "requirement", "enabledByDefault": true },
        ]);
        let defaults = default_options(manifest_options.as_array().unwrap());
        let tmp = folder_with("{ \"loadOrder\": [] }");
        assert!(set_enabled(tmp.path(), "M", true, Some(&defaults), None).unwrap());
        assert_eq!(read(tmp.path())["modOptions"]["M"], serde_json::json!(["Darker", "Hands:Bare hands"]));

        // A mod that already has options keeps them.
        let tmp = folder_with(SMF_WRITTEN);
        assert!(set_enabled(tmp.path(), "B", false, None, None).unwrap());
        assert!(set_enabled(tmp.path(), "B", true, Some(&defaults), None).unwrap());
        assert_eq!(read(tmp.path())["modOptions"]["B"], serde_json::json!(["Hands — Signature Suit:Bare hands"]));
    }

    #[test]
    fn keeps_a_copy_before_each_write_and_prunes_old_ones() {
        let tmp = folder_with(SMF_WRITTEN);
        let backups = tmp.path().join("backups");
        std::fs::create_dir_all(&backups).unwrap();
        for i in 0..BACKUPS_KEPT {
            std::fs::write(backups.join(format!("config-{i:015}.json")), "old").unwrap();
        }
        std::fs::write(backups.join("unrelated.txt"), "keep me").unwrap();
        assert!(set_enabled(tmp.path(), "C", true, None, Some(&backups)).unwrap());
        let mut names: Vec<String> = std::fs::read_dir(&backups)
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        assert_eq!(names.len(), BACKUPS_KEPT + 1, "{names:?}");
        assert!(!names.contains(&format!("config-{:015}.json", 0)));
        assert!(names.contains(&"unrelated.txt".to_string()));
        let newest = names.iter().filter(|n| n.starts_with("config-")).max().unwrap();
        assert_eq!(std::fs::read_to_string(backups.join(newest)).unwrap(), SMF_WRITTEN);
    }

    #[test]
    fn a_failed_backup_stops_the_write() {
        let tmp = folder_with(SMF_WRITTEN);
        let blocker = tmp.path().join("not-a-folder");
        std::fs::write(&blocker, "").unwrap();
        let err = set_enabled(tmp.path(), "C", true, None, Some(&blocker)).unwrap_err();
        assert!(err.contains("Nothing was changed"), "{err}");
        assert_eq!(std::fs::read_to_string(tmp.path().join("config.json")).unwrap(), SMF_WRITTEN);
    }

    #[test]
    fn refuses_a_config_it_cannot_read() {
        for bad in ["{ not json", "[1, 2]", "{ \"loadOrder\": \"A\" }"] {
            let tmp = folder_with(bad);
            assert!(set_enabled(tmp.path(), "C", true, None, None).is_err(), "{bad}");
            assert_eq!(std::fs::read_to_string(tmp.path().join("config.json")).unwrap(), bad);
        }
    }

    #[test]
    fn a_change_made_meanwhile_stops_the_write() {
        let tmp = folder_with("{loadOrder:['Elsewhere']}");
        let path = tmp.path().join("config.json");
        // This process read SMF_WRITTEN; another program has saved since.
        let err = replace_if_unchanged(&path, SMF_WRITTEN, b"{}").unwrap_err();
        assert!(err.contains("Another program"), "{err}");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "{loadOrder:['Elsewhere']}");
        assert_eq!(std::fs::read_dir(tmp.path()).unwrap().count(), 1, "the temporary file was left behind");

        replace_if_unchanged(&path, "{loadOrder:['Elsewhere']}", b"{}").unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "{}");
        assert_eq!(std::fs::read_dir(tmp.path()).unwrap().count(), 1);
    }

    #[test]
    fn the_result_reads_back_as_json5() {
        let tmp = folder_with(SMF_WRITTEN);
        set_enabled(tmp.path(), "C", true, None, None).unwrap();
        let text = std::fs::read_to_string(tmp.path().join("config.json")).unwrap();
        let back: Value = json5::from_str(&text).unwrap();
        assert_eq!(back, read(tmp.path()));
    }
}
