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

/// A hash of everything a deploy reads that this app can change or see
/// change: `config.json`, and each mod folder's name, its manifest's time
/// and its own time. Equal fingerprints mean nothing changed since.
pub fn fingerprint(smf_folder: &Path) -> Result<String, String> {
    let mut h = Sha256::new();
    let config = smf_folder.join("config.json");
    h.update(std::fs::read(&config).map_err(|e| format!("Could not read {}: {e}", config.display()))?);
    let mods = smf_folder.join("Mods");
    let mut entries: Vec<(String, String)> = std::fs::read_dir(&mods)
        .map_err(|e| format!("Could not read {}: {e}", mods.display()))?
        .flatten()
        .map(|e| {
            let p = e.path();
            let time = |q: &Path| {
                std::fs::metadata(q)
                    .and_then(|m| m.modified())
                    .ok()
                    .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                    .map_or(0, |d| d.as_nanos())
            };
            (e.file_name().to_string_lossy().into_owned(), format!("{}:{}", time(&p), time(&p.join("manifest.json"))))
        })
        .collect();
    entries.sort();
    for (name, times) in entries {
        h.update(name.as_bytes());
        h.update(b"\0");
        h.update(times.as_bytes());
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
        assert_ne!(b, fingerprint(tmp.path()).unwrap());
    }
}
