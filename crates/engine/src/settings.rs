//! The app's own settings, `settings.json` in its data folder. Today it holds
//! only the mod types the user chose over the ones worked out from files.
//! These never reach SMF.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde_json::{Map, Value};
use vmm_core::backend::ModKind;
use vmm_core::error::{ErrorCode, VmmError};

const FILE: &str = "settings.json";

/// Where the settings file is, in the data folder `dir`.
pub fn path(dir: &Path) -> PathBuf {
    dir.join(FILE)
}

/// The settings, plus every key this version does not know, so a newer
/// version's settings survive a save by this one.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Settings {
    /// Mod id (as SMF's load order names it) to the type the user chose.
    pub kind_overrides: BTreeMap<String, ModKind>,
    /// Settings' "Let AI tools make changes". Off unless the user turns it
    /// on; while off, AI tools can read and plan but not apply.
    pub ai_changes: bool,
    rest: Map<String, Value>,
}

impl Settings {
    /// Reads `settings.json` from `dir`. A missing file is empty settings; a
    /// file that can't be read is an error, so it is never overwritten.
    pub fn load(dir: &Path) -> Result<Settings, VmmError> {
        let path = dir.join(FILE);
        let text = match std::fs::read_to_string(&path) {
            Ok(t) => t,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Settings::default()),
            Err(e) => {
                return Err(VmmError::new(
                    ErrorCode::SettingsUnreadable,
                    format!("Could not read {}: {e}", path.display()),
                ));
            }
        };
        let unreadable = |why: String| {
            VmmError::new(
                ErrorCode::SettingsUnreadable,
                format!(
                    "Vaexil Mod Manager's settings file can't be read ({why}), so type changes are off until it is fixed or removed: {}",
                    path.display()
                ),
            )
        };
        let mut rest = match serde_json::from_str::<Value>(text.trim_start_matches('\u{feff}')) {
            Ok(Value::Object(map)) => map,
            Ok(_) => return Err(unreadable("it is not a settings object".into())),
            Err(e) => return Err(unreadable(e.to_string())),
        };
        let mut kind_overrides = BTreeMap::new();
        if let Some(found) = rest.remove("kindOverrides") {
            let Value::Object(found) = found else { return Err(unreadable("kindOverrides is not a list".into())) };
            for (id, kind) in found {
                // An unknown type is dropped rather than failing the whole file.
                if let Some(kind) = kind.as_str().and_then(ModKind::from_key) {
                    kind_overrides.insert(id, kind);
                }
            }
        }
        let ai_changes = rest.remove("aiChanges").and_then(|v| v.as_bool()).unwrap_or(false);
        Ok(Settings { kind_overrides, ai_changes, rest })
    }

    /// Writes `settings.json` in `dir`, replacing it in one rename.
    pub fn save(&self, dir: &Path) -> Result<(), VmmError> {
        let fail = |e: std::io::Error| {
            VmmError::new(
                ErrorCode::SaveFailed,
                format!("Could not save Vaexil Mod Manager's settings in {}: {e}", dir.display()),
            )
        };
        std::fs::create_dir_all(dir).map_err(fail)?;
        let mut map = self.rest.clone();
        map.insert(
            "kindOverrides".into(),
            Value::Object(
                self.kind_overrides.iter().map(|(id, k)| (id.clone(), Value::String(k.key().into()))).collect(),
            ),
        );
        if self.ai_changes {
            map.insert("aiChanges".into(), Value::Bool(true));
        }
        let mut text = serde_json::to_string_pretty(&Value::Object(map)).expect("a JSON object always serializes");
        text.push('\n');
        let temp: PathBuf = dir.join(format!("{FILE}.{}.tmp", std::process::id()));
        let result = std::fs::write(&temp, text).and_then(|()| std::fs::rename(&temp, dir.join(FILE)));
        if result.is_err() {
            let _ = std::fs::remove_file(&temp);
        }
        result.map_err(fail)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_file_is_empty_settings() {
        let tmp = tempfile::tempdir().unwrap();
        assert_eq!(Settings::load(tmp.path()).unwrap(), Settings::default());
    }

    #[test]
    fn saves_and_loads_overrides_and_keeps_unknown_keys() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(
            tmp.path().join(FILE),
            r#"{ "future": { "x": 1 }, "kindOverrides": { "A": "sound", "B": "not-a-kind" } }"#,
        )
        .unwrap();
        let mut s = Settings::load(tmp.path()).unwrap();
        assert_eq!(s.kind_overrides.get("A"), Some(&ModKind::Sound));
        assert!(!s.kind_overrides.contains_key("B"));
        s.kind_overrides.insert("C".into(), ModKind::ModelsTextures);
        s.save(tmp.path()).unwrap();

        let back = Settings::load(tmp.path()).unwrap();
        assert_eq!(back, s);
        let text = std::fs::read_to_string(tmp.path().join(FILE)).unwrap();
        assert!(text.contains("\"future\"") && text.contains("\"models-textures\""), "{text}");
        let leftovers = std::fs::read_dir(tmp.path()).unwrap().count();
        assert_eq!(leftovers, 1);
    }

    #[test]
    fn an_unreadable_file_is_an_error_and_is_left_alone() {
        let tmp = tempfile::tempdir().unwrap();
        for bad in ["{ not json", "[1]", r#"{ "kindOverrides": [] }"#] {
            std::fs::write(tmp.path().join(FILE), bad).unwrap();
            let err = Settings::load(tmp.path()).unwrap_err();
            assert!(err.message.contains("type changes are off"), "{err}");
            assert_eq!(std::fs::read_to_string(tmp.path().join(FILE)).unwrap(), bad);
        }
    }
}
