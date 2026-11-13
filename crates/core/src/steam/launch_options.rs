//! Reads and edits `LaunchOptions` in a Steam account's `localconfig.vdf`.
//!
//! Edits are surgical: only the bytes of one value change. Steam rewrites
//! this file while it runs, so callers must only write when Steam is closed.

use std::path::Path;

use super::vdf::{self, Value, VdfError};

const APPS_PATH: [&str; 5] = ["UserLocalConfigStore", "Software", "Valve", "Steam", "apps"];

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum LaunchOptionsError {
    #[error(transparent)]
    Parse(#[from] VdfError),
    #[error("this localconfig.vdf has no Software/Valve/Steam/apps section; it was left unchanged")]
    UnexpectedLayout,
}

fn apps(doc: &Value) -> Option<&Value> {
    doc.path(&APPS_PATH)
}

pub fn get_launch_options(text: &str, app_id: u32) -> Result<String, LaunchOptionsError> {
    let doc = vdf::parse(text)?;
    let apps = apps(&doc).ok_or(LaunchOptionsError::UnexpectedLayout)?;
    Ok(apps.path(&[&app_id.to_string(), "LaunchOptions"]).and_then(Value::as_str).unwrap_or_default().to_string())
}

/// The app's own file name; launch options that start it are ours.
pub const APP_EXE: &str = "vaexil-mod-manager.exe";
/// The argument the app takes when Steam's Play button starts it.
pub const STEAM_PLAY_ARG: &str = "--steam-play";

/// Whether launch options start Vaexil Mod Manager for Steam's Play button
/// (`"…\vaexil-mod-manager.exe" --steam-play -- %command%`).
pub fn is_ours(value: &str) -> bool {
    let args = crate::cmdline::split_command_line(value);
    args.first().is_some_and(|p| crate::paths::file_name_eq(Path::new(p), APP_EXE))
        && args.iter().any(|a| a == STEAM_PLAY_ARG)
}

/// Returns the new file text with `LaunchOptions` for `app_id` set to `value`.
pub fn set_launch_options(text: &str, app_id: u32, value: &str) -> Result<String, LaunchOptionsError> {
    let doc = vdf::parse(text)?;
    let apps_node = apps(&doc).ok_or(LaunchOptionsError::UnexpectedLayout)?;
    let quoted = format!("\"{}\"", vdf::escape(value));
    let id = app_id.to_string();

    let replace = |at: std::ops::Range<usize>, with: &str| {
        let mut out = String::with_capacity(text.len() + with.len());
        out.push_str(&text[..at.start]);
        out.push_str(with);
        out.push_str(&text[at.end..]);
        out
    };

    match apps_node.get(&id) {
        Some(app @ Value::Obj { close, children, .. }) => {
            if let Some(Value::Str { span, .. }) = app.get("LaunchOptions") {
                return Ok(replace(span.clone(), &quoted));
            }
            let indent = match children.first() {
                Some(child) => vdf::line_indent(text, child.start).to_string(),
                None => format!("{}\t", vdf::line_indent(text, *close)),
            };
            let insert_at = text[..*close].rfind('\n').map_or(*close, |i| i + 1);
            let line = format!("{indent}\"LaunchOptions\"\t\t{quoted}\n");
            Ok(replace(insert_at..insert_at, &line))
        }
        Some(Value::Str { .. }) => Err(LaunchOptionsError::UnexpectedLayout),
        None => {
            let Value::Obj { close, children, .. } = apps_node else {
                return Err(LaunchOptionsError::UnexpectedLayout);
            };
            let indent = match children.first() {
                Some(child) => vdf::line_indent(text, child.start).to_string(),
                None => format!("{}\t", vdf::line_indent(text, *close)),
            };
            let insert_at = text[..*close].rfind('\n').map_or(*close, |i| i + 1);
            let block = format!("{indent}\"{id}\"\n{indent}{{\n{indent}\t\"LaunchOptions\"\t\t{quoted}\n{indent}}}\n");
            Ok(replace(insert_at..insert_at, &block))
        }
    }
}

/// The launch options that make Steam's Play button start the game through
/// Vaexil Mod Manager. Steam replaces `%command%` with the game's own command,
/// which the app runs itself, so Play never loops back through Steam.
pub fn wrapper_launch_options(app_exe: &Path) -> String {
    format!("\"{}\" {STEAM_PLAY_ARG} -- %command%", app_exe.display())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RestorePlan {
    /// The value is still the app's; write this text to restore the
    /// game's previous launch options.
    Restore(String),
    /// Someone changed the value since the app set it; leave it alone.
    ChangedElsewhere,
}

/// Decides how to undo the app's Steam Play hook in `text`.
pub fn plan_restore(text: &str, app_id: u32, applied: &str, previous: &str) -> Result<RestorePlan, LaunchOptionsError> {
    if get_launch_options(text, app_id)? != applied {
        return Ok(RestorePlan::ChangedElsewhere);
    }
    Ok(RestorePlan::Restore(set_launch_options(text, app_id, previous)?))
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "\"UserLocalConfigStore\"\n{\n\t\"Software\"\n\t{\n\t\t\"Valve\"\n\t\t{\n\t\t\t\"Steam\"\n\t\t\t{\n\t\t\t\t\"apps\"\n\t\t\t\t{\n\t\t\t\t\t\"292030\"\n\t\t\t\t\t{\n\t\t\t\t\t\t\"LastPlayed\"\t\t\"1791973686\"\n\t\t\t\t\t}\n\t\t\t\t\t\"1091500\"\n\t\t\t\t\t{\n\t\t\t\t\t\t\"LaunchOptions\"\t\t\"-skipStartScreen\"\n\t\t\t\t\t\t\"Playtime\"\t\t\"5000\"\n\t\t\t\t\t}\n\t\t\t\t}\n\t\t\t}\n\t\t}\n\t}\n\t\"friends\"\n\t{\n\t}\n}\n";

    #[test]
    fn reads_existing_and_missing() {
        assert_eq!(get_launch_options(SAMPLE, 1091500).unwrap(), "-skipStartScreen");
        assert_eq!(get_launch_options(SAMPLE, 292030).unwrap(), "");
        assert_eq!(get_launch_options(SAMPLE, 1).unwrap(), "");
    }

    #[test]
    fn replaces_existing_value_only() {
        let v = wrapper_launch_options(Path::new(
            r"C:\Users\me\AppData\Local\Programs\Vaexil Mod Manager\vaexil-mod-manager.exe",
        ));
        let out = set_launch_options(SAMPLE, 1091500, &v).unwrap();
        assert_eq!(get_launch_options(&out, 1091500).unwrap(), v);
        let changed: Vec<_> = SAMPLE.lines().zip(out.lines()).filter(|(a, b)| a != b).collect();
        assert_eq!(changed.len(), 1);
        assert_eq!(out.lines().count(), SAMPLE.lines().count());
    }

    #[test]
    fn inserts_into_app_without_options() {
        let out = set_launch_options(SAMPLE, 292030, "--x").unwrap();
        assert_eq!(get_launch_options(&out, 292030).unwrap(), "--x");
        assert!(out.contains("\t\t\t\t\t\t\"LaunchOptions\"\t\t\"--x\"\n\t\t\t\t\t}"));
        assert_eq!(out.lines().count(), SAMPLE.lines().count() + 1);
        // Everything else is untouched.
        let restored = out.replace("\t\t\t\t\t\t\"LaunchOptions\"\t\t\"--x\"\n", "");
        assert_eq!(restored, SAMPLE);
    }

    #[test]
    fn creates_missing_app_block() {
        let out = set_launch_options(SAMPLE, 499450, "-y").unwrap();
        assert_eq!(get_launch_options(&out, 499450).unwrap(), "-y");
        assert_eq!(get_launch_options(&out, 1091500).unwrap(), "-skipStartScreen");
        vdf::parse(&out).unwrap();
    }

    #[test]
    fn restoring_previous_value_round_trips() {
        let changed = set_launch_options(SAMPLE, 1091500, "\"C:\\x y\\l.exe\" --steam a -- %command%").unwrap();
        let restored = set_launch_options(&changed, 1091500, "-skipStartScreen").unwrap();
        assert_eq!(restored, SAMPLE);
    }

    #[test]
    fn refuses_unexpected_files() {
        assert_eq!(set_launch_options("\"Other\" { }", 1, "x"), Err(LaunchOptionsError::UnexpectedLayout));
        assert!(matches!(set_launch_options("\"broken\" {", 1, "x"), Err(LaunchOptionsError::Parse(_))));
    }

    #[test]
    fn plan_restore_puts_back_previous_value() {
        let applied = "\"C:\\x\\l.exe\" --steam a -- %command%";
        let hooked = set_launch_options(SAMPLE, 1091500, applied).unwrap();
        match plan_restore(&hooked, 1091500, applied, "-skipStartScreen").unwrap() {
            RestorePlan::Restore(text) => assert_eq!(text, SAMPLE),
            other => panic!("expected a restore, got {other:?}"),
        }
        // A game that had no options before gets an empty value back.
        let hooked = set_launch_options(SAMPLE, 292030, applied).unwrap();
        let RestorePlan::Restore(text) = plan_restore(&hooked, 292030, applied, "").unwrap() else { panic!() };
        assert_eq!(get_launch_options(&text, 292030).unwrap(), "");
    }

    #[test]
    fn plan_restore_leaves_values_changed_elsewhere() {
        let applied = "\"C:\\x\\l.exe\" --steam a -- %command%";
        let edited = set_launch_options(SAMPLE, 1091500, "-windowed").unwrap();
        assert_eq!(plan_restore(&edited, 1091500, applied, "").unwrap(), RestorePlan::ChangedElsewhere);
        assert!(plan_restore("\"Other\" { }", 1, applied, "").is_err());
    }

    #[test]
    fn wrapper_quotes_the_path_and_is_recognised() {
        let ours = wrapper_launch_options(Path::new(r"C:\Program Files\VMM\vaexil-mod-manager.exe"));
        assert_eq!(ours, r#""C:\Program Files\VMM\vaexil-mod-manager.exe" --steam-play -- %command%"#);
        assert!(is_ours(&ours));
        assert!(is_ours(r#"C:\x\Vaexil-Mod-Manager.EXE --steam-play -- %command%"#));
        assert!(!is_ours("-skipStartScreen"));
        assert!(!is_ours(r#""C:\x\other.exe" --steam-play -- %command%"#));
        assert!(!is_ours(r#""C:\x\vaexil-mod-manager.exe" -- %command%"#));
        assert!(!is_ours(""));
    }
}
