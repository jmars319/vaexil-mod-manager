//! The app's event log (ADR 0003, section 4): one JSON object per line in
//! `logs\vmm-<YYYY-MM-DD>.jsonl` in the data folder, a file per day (UTC),
//! kept for 14 days. Every change and every failed operation is recorded with
//! who made it (`gui`, `cli` or `mcp:<AI app>`). Logging is best effort: a
//! line that can't be written never stops the operation it describes.

use std::io::Write;
use std::path::{Path, PathBuf};

use vmm_core::schema::LogEvent;

/// How many days of event logs are kept.
pub const KEEP_DAYS: u64 = 14;
const DAY_MS: u64 = 24 * 60 * 60 * 1000;

/// `YYYY-MM-DD` (UTC) for a time in milliseconds since 1970.
pub fn date(ts_ms: u64) -> String {
    // Howard Hinnant's days-to-civil algorithm.
    let z = (ts_ms / DAY_MS) as i64 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02}")
}

/// `YYYY-MM-DD HH:MM:SS` (UTC).
pub fn timestamp(ts_ms: u64) -> String {
    let secs = (ts_ms % DAY_MS) / 1000;
    format!("{} {:02}:{:02}:{:02}", date(ts_ms), secs / 3600, secs / 60 % 60, secs % 60)
}

fn file_for(ts_ms: u64) -> String {
    format!("vmm-{}.jsonl", date(ts_ms))
}

/// The event log files in `dir`, oldest first.
fn files(dir: &Path) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = std::fs::read_dir(dir)
        .map(|rd| {
            rd.flatten()
                .map(|e| e.path())
                .filter(|p| {
                    p.file_name()
                        .and_then(|n| n.to_str())
                        .is_some_and(|n| n.starts_with("vmm-") && n.ends_with(".jsonl"))
                })
                .collect()
        })
        .unwrap_or_default();
    out.sort();
    out
}

pub fn append(dir: &Path, event: &LogEvent) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    let mut line = serde_json::to_string(event).expect("an event serializes");
    line.push('\n');
    std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(dir.join(file_for(event.ts_ms)))?
        .write_all(line.as_bytes())
}

/// Deletes event logs older than [`KEEP_DAYS`]. Best effort.
pub fn prune(dir: &Path, now_ms: u64) {
    let oldest = file_for(now_ms.saturating_sub(KEEP_DAYS * DAY_MS));
    for f in files(dir) {
        if f.file_name().and_then(|n| n.to_str()).is_some_and(|n| n < oldest.as_str()) {
            let _ = std::fs::remove_file(f);
        }
    }
}

/// The newest `limit` events, newest last. Lines that can't be read are skipped.
pub fn recent(dir: &Path, limit: usize) -> Vec<LogEvent> {
    let mut out: Vec<LogEvent> = Vec::new();
    for f in files(dir).into_iter().rev() {
        let text = std::fs::read_to_string(&f).unwrap_or_default();
        let mut day: Vec<LogEvent> = text.lines().filter_map(|l| serde_json::from_str(l).ok()).collect();
        day.append(&mut out);
        out = day;
        if out.len() >= limit {
            break;
        }
    }
    let skip = out.len().saturating_sub(limit);
    out.split_off(skip)
}

#[cfg(test)]
mod tests {
    use vmm_core::schema::LogLevel;

    use super::*;

    fn event(ts_ms: u64, name: &str) -> LogEvent {
        LogEvent {
            ts_ms,
            level: LogLevel::Info,
            event: name.into(),
            caller: "cli".into(),
            game: None,
            backend: None,
            mod_id: None,
            plan: None,
            message: String::new(),
            error: None,
        }
    }

    #[test]
    fn dates_are_utc_calendar_days() {
        assert_eq!(date(0), "1970-01-01");
        assert_eq!(date(951_782_400_000), "2000-02-29");
        assert_eq!(date(1_794_560_376_416), "2026-11-13");
        assert_eq!(timestamp(1_794_560_376_416), "2026-11-13 08:59:36");
    }

    #[test]
    fn appends_by_day_reads_newest_last_and_prunes_old_days() {
        let tmp = tempfile::tempdir().unwrap();
        let day = DAY_MS;
        let now = 1_794_560_376_416;
        for (i, ts) in [now - 20 * day, now - 2 * day, now - day, now].into_iter().enumerate() {
            append(tmp.path(), &event(ts, &format!("e{i}"))).unwrap();
        }
        append(tmp.path(), &event(now + 1, "e4")).unwrap();
        let names = |v: Vec<LogEvent>| v.into_iter().map(|e| e.event).collect::<Vec<_>>();
        assert_eq!(names(recent(tmp.path(), 3)), ["e2", "e3", "e4"]);
        prune(tmp.path(), now);
        assert_eq!(names(recent(tmp.path(), 10)), ["e1", "e2", "e3", "e4"]);
    }
}
