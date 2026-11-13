//! A crash record (ADR 0003, section 4): if the app or `vmm` panics, a
//! `crash-<time>.json` is written to the data folder before the process ends,
//! so `vmm diagnose` and a bug report can show what happened. The record
//! holds the panic message and where it happened, never file contents.

use std::path::{Path, PathBuf};

use serde_json::json;

use crate::deploy::now_ms;

/// Installs the panic hook. The previous hook still runs after it.
pub fn install(data_dir: Option<PathBuf>, caller: String) {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        if let Some(dir) = &data_dir {
            let message = info
                .payload()
                .downcast_ref::<&str>()
                .map(|s| s.to_string())
                .or_else(|| info.payload().downcast_ref::<String>().cloned())
                .unwrap_or_else(|| "a panic with no message".into());
            let record = json!({
                "tsMs": now_ms(),
                "caller": caller,
                "appVersion": env!("CARGO_PKG_VERSION"),
                "thread": std::thread::current().name().unwrap_or("unnamed"),
                "message": message,
                "location": info.location().map(|l| format!("{}:{}", l.file(), l.line())),
            });
            let _ = std::fs::create_dir_all(dir);
            let _ = std::fs::write(
                dir.join(format!("crash-{}.json", now_ms())),
                serde_json::to_string_pretty(&record).unwrap_or_default(),
            );
        }
        previous(info);
    }));
}

/// The crash records in `dir`, newest first.
pub fn records(dir: &Path) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = std::fs::read_dir(dir)
        .map(|rd| {
            rd.flatten()
                .map(|e| e.path())
                .filter(|p| {
                    p.file_name()
                        .and_then(|n| n.to_str())
                        .is_some_and(|n| n.starts_with("crash-") && n.ends_with(".json"))
                })
                .collect()
        })
        .unwrap_or_default();
    out.sort();
    out.reverse();
    out
}
