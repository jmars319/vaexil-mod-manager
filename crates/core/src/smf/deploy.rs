//! Running SMF's own `Deploy.exe`, exactly as SMF's Mod Manager runs it
//! (`--doNotPause --colors`, from SMF's folder), and telling whether anything
//! changed since the last deploy. `Deploy.exe` itself is never changed.

use std::io::{BufRead, BufReader};
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::mpsc;

use sha2::{Digest, Sha256};

/// The flags SMF's own Mod Manager passes.
pub const ARGS: [&str; 2] = ["--doNotPause", "--colors"];

pub use crate::framework::DeployOutcome;

/// Runs `program` with `args` in `cwd`, passing each line of its output
/// (stdout and stderr, colour codes removed) to `on_line` as it arrives.
pub fn run_tool(
    program: &Path,
    args: &[&str],
    cwd: &Path,
    on_line: &mut dyn FnMut(&str),
) -> Result<DeployOutcome, String> {
    let mut cmd = Command::new(program);
    cmd.args(args).current_dir(cwd).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // CREATE_NO_WINDOW: no console window flashes up.
        cmd.creation_flags(0x0800_0000);
    }
    let mut child = cmd.spawn().map_err(|e| format!("Could not start {}: {e}", program.display()))?;
    let (tx, rx) = mpsc::channel::<String>();
    let mut readers = Vec::new();
    for stream in [
        child.stdout.take().map(|s| Box::new(s) as Box<dyn std::io::Read + Send>),
        child.stderr.take().map(|s| Box::new(s) as _),
    ]
    .into_iter()
    .flatten()
    {
        let tx = tx.clone();
        readers.push(std::thread::spawn(move || {
            for line in BufReader::new(stream).split(b'\n').map_while(Result::ok) {
                let text = String::from_utf8_lossy(&line);
                let _ = tx.send(strip_ansi(text.trim_end_matches('\r')));
            }
        }));
    }
    drop(tx);
    for line in rx {
        on_line(&line);
    }
    for r in readers {
        let _ = r.join();
    }
    let status = child.wait().map_err(|e| format!("Could not wait for {}: {e}", program.display()))?;
    Ok(DeployOutcome { success: status.success(), exit_code: status.code() })
}

/// Runs SMF's `Deploy.exe` in `smf_folder`.
pub fn deploy(smf_folder: &Path, on_line: &mut dyn FnMut(&str)) -> Result<DeployOutcome, String> {
    run_tool(&smf_folder.join("Deploy.exe"), &ARGS, smf_folder, on_line)
}

/// Removes terminal colour codes (`ESC [ ... letter`).
pub fn strip_ansi(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\u{1b}' && chars.peek() == Some(&'[') {
            chars.next();
            for d in chars.by_ref() {
                if d.is_ascii_alphabetic() {
                    break;
                }
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// A cheap check for [`fingerprint`]'s cache: `config.json`'s bytes and the
/// top level of `Mods` (names and times). It changes with anything the app
/// itself writes, and with mods being added or removed, but not with a file
/// changed deep inside a mod.
pub fn quick_key(smf_folder: &Path) -> String {
    let mut h = Sha256::new();
    h.update(std::fs::read(smf_folder.join("config.json")).unwrap_or_default());
    let mut top: Vec<String> = std::fs::read_dir(smf_folder.join("Mods"))
        .map(|list| {
            list.flatten()
                .map(|e| {
                    let time = e
                        .metadata()
                        .and_then(|m| m.modified())
                        .ok()
                        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                        .map_or(0, |d| d.as_nanos());
                    format!("{}\0{time}", e.file_name().to_string_lossy())
                })
                .collect()
        })
        .unwrap_or_default();
    top.sort();
    for t in top {
        h.update(t.as_bytes());
        h.update(b"\n");
    }
    h.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

/// A hash of everything a deploy reads: `config.json`'s bytes, and every
/// file under `Mods` by path, size and modified time, so a file changed or
/// replaced deep inside a mod counts. Equal fingerprints mean nothing changed
/// since. Walking every mod takes about a tenth of a second on JASON-PC.
pub fn fingerprint(smf_folder: &Path) -> Result<String, String> {
    let mut h = Sha256::new();
    let config = smf_folder.join("config.json");
    h.update(std::fs::read(&config).map_err(|e| format!("Could not read {}: {e}", config.display()))?);
    let mods = smf_folder.join("Mods");
    if !mods.is_dir() {
        return Err(format!("Could not read {}: it is not a folder", mods.display()));
    }
    let mut entries: Vec<String> = Vec::new();
    let mut stack = vec![mods.clone()];
    while let Some(dir) = stack.pop() {
        let list = std::fs::read_dir(&dir).map_err(|e| format!("Could not read {}: {e}", dir.display()))?;
        for e in list.flatten() {
            let path = e.path();
            let rel = path.strip_prefix(&mods).unwrap_or(&path).to_string_lossy().into_owned();
            let Ok(meta) = e.metadata() else { continue };
            if meta.is_dir() {
                entries.push(format!("{rel}/"));
                stack.push(path);
            } else {
                let time = meta
                    .modified()
                    .ok()
                    .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                    .map_or(0, |d| d.as_nanos());
                entries.push(format!("{rel}\0{}\0{time}", meta.len()));
            }
        }
    }
    entries.sort();
    for e in entries {
        h.update(e.as_bytes());
        h.update(b"\n");
    }
    Ok(h.finalize().iter().map(|b| format!("{b:02x}")).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_colour_codes() {
        assert_eq!(strip_ansi("\u{1b}[32mINFO\u{1b}[0m\tDeploy\tDone"), "INFO\tDeploy\tDone");
        assert_eq!(strip_ansi("plain"), "plain");
    }

    #[cfg(windows)]
    #[test]
    fn streams_output_and_reports_the_exit_code() {
        let tmp = tempfile::tempdir().unwrap();
        let cmd = Path::new(r"C:\Windows\System32\cmd.exe");
        let mut lines = Vec::new();
        let ok =
            run_tool(cmd, &["/c", "echo first&& echo second 1>&2"], tmp.path(), &mut |l| lines.push(l.to_string()))
                .unwrap();
        assert!(ok.success);
        lines.sort();
        assert_eq!(lines, ["first", "second "]);
        let failed = run_tool(cmd, &["/c", "exit 3"], tmp.path(), &mut |_| {}).unwrap();
        assert_eq!(failed, DeployOutcome { success: false, exit_code: Some(3) });
        assert!(run_tool(&tmp.path().join("missing.exe"), &[], tmp.path(), &mut |_| {}).is_err());
    }

    #[test]
    fn the_fingerprint_follows_config_and_mods() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(tmp.path().join("Mods/A")).unwrap();
        std::fs::write(tmp.path().join("config.json"), "{ loadOrder: [] }").unwrap();
        let a = fingerprint(tmp.path()).unwrap();
        assert_eq!(a, fingerprint(tmp.path()).unwrap());
        std::fs::write(tmp.path().join("config.json"), "{ loadOrder: ['A'] }").unwrap();
        let b = fingerprint(tmp.path()).unwrap();
        assert_ne!(a, b);
        std::fs::create_dir_all(tmp.path().join("Mods/B")).unwrap();
        let c = fingerprint(tmp.path()).unwrap();
        assert_ne!(b, c);
        // A file changed deep inside a mod counts, even when no folder's own
        // time moves.
        let deep = tmp.path().join("Mods/A/content/chunk0/00AA00AA00AA00AA.TEMP");
        std::fs::create_dir_all(deep.parent().unwrap()).unwrap();
        std::fs::write(&deep, b"one").unwrap();
        let d = fingerprint(tmp.path()).unwrap();
        assert_ne!(c, d);
        std::fs::write(&deep, b"two!").unwrap();
        assert_ne!(d, fingerprint(tmp.path()).unwrap());
    }
}
