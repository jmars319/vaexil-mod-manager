//! The app's own JSON files in its data folder, read and written whole. A
//! missing file reads as empty; a file that can't be read is an error, and is
//! then never overwritten. Writes replace the file in one rename.

use std::path::Path;

use serde_json::{Map, Value};
use vmm_core::error::{ErrorCode, VmmError};

pub fn load(dir: &Path, file: &str) -> Result<Map<String, Value>, VmmError> {
    let path = dir.join(file);
    let text = match std::fs::read_to_string(&path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Map::new()),
        Err(e) => {
            return Err(VmmError::new(ErrorCode::SettingsUnreadable, format!("Could not read {}: {e}", path.display())));
        }
    };
    match serde_json::from_str::<Value>(text.trim_start_matches('\u{feff}')) {
        Ok(Value::Object(map)) => Ok(map),
        Ok(_) => Err(VmmError::new(
            ErrorCode::SettingsUnreadable,
            format!("{} does not hold a settings object. Fix or remove it.", path.display()),
        )),
        Err(e) => Err(VmmError::new(
            ErrorCode::SettingsUnreadable,
            format!("{} can't be read ({e}). Fix or remove it.", path.display()),
        )),
    }
}

pub fn save(dir: &Path, file: &str, map: &Map<String, Value>) -> Result<(), VmmError> {
    let fail = |e: std::io::Error| {
        VmmError::new(ErrorCode::SaveFailed, format!("Could not save {} in {}: {e}", file, dir.display()))
    };
    std::fs::create_dir_all(dir).map_err(fail)?;
    let mut text = serde_json::to_string_pretty(&Value::Object(map.clone())).expect("a JSON object always serializes");
    text.push('\n');
    let temp = dir.join(format!("{file}.{}.tmp", std::process::id()));
    let result = std::fs::write(&temp, text).and_then(|()| std::fs::rename(&temp, dir.join(file)));
    if result.is_err() {
        let _ = std::fs::remove_file(&temp);
    }
    result.map_err(fail)
}

pub fn strings(v: Option<&Value>) -> Vec<String> {
    v.and_then(Value::as_array)
        .map(|a| a.iter().filter_map(Value::as_str).map(str::to_string).collect())
        .unwrap_or_default()
}

pub fn to_strings(list: &[String]) -> Value {
    Value::Array(list.iter().cloned().map(Value::String).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_and_refuses_what_it_cannot_read() {
        let tmp = tempfile::tempdir().unwrap();
        assert!(load(tmp.path(), "x.json").unwrap().is_empty());
        let mut m = Map::new();
        m.insert("a".into(), to_strings(&["1".into(), "2".into()]));
        save(tmp.path(), "x.json", &m).unwrap();
        assert_eq!(strings(load(tmp.path(), "x.json").unwrap().get("a")), ["1", "2"]);
        assert_eq!(std::fs::read_dir(tmp.path()).unwrap().count(), 1);
        std::fs::write(tmp.path().join("x.json"), "[1]").unwrap();
        assert!(load(tmp.path(), "x.json").is_err());
    }
}
