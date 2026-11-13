//! Who changed what (ADR 0003, section 5): every applied plan is appended to
//! `journal.jsonl` in the data folder, one JSON object per line, with the
//! caller and, for an AI tool, the user's request it was made for.
//!
//! The journal is a record, not a lock: failing to write it never undoes or
//! blocks a change that has been made, but it is reported.

use std::io::Write;
use std::path::Path;

use serde::Serialize;
use vmm_core::schema::PlanStep;

use crate::deploy;

const FILE: &str = "journal.jsonl";

/// Who is asking the engine for a change.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Caller {
    Window,
    Cli,
    /// An AI tool through `vmm mcp`, by the name its client gave.
    Mcp {
        client: String,
    },
}

impl Caller {
    pub fn label(&self) -> String {
        match self {
            Caller::Window => "gui".into(),
            Caller::Cli => "cli".into(),
            Caller::Mcp { client } => format!("mcp:{client}"),
        }
    }

    pub fn is_ai(&self) -> bool {
        matches!(self, Caller::Mcp { .. })
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Entry<'a> {
    ts_ms: u64,
    caller: String,
    game: &'a str,
    plan_id: &'a str,
    request: Option<&'a str>,
    steps: &'a [PlanStep],
}

/// Appends one applied plan to the journal.
pub fn append(
    dir: &Path,
    caller: &Caller,
    game: &str,
    plan_id: &str,
    request: Option<&str>,
    steps: &[PlanStep],
) -> std::io::Result<()> {
    let entry = Entry { ts_ms: deploy::now_ms(), caller: caller.label(), game, plan_id, request, steps };
    let mut line = serde_json::to_string(&entry).expect("a journal entry serializes");
    line.push('\n');
    std::fs::create_dir_all(dir)?;
    std::fs::OpenOptions::new().create(true).append(true).open(dir.join(FILE))?.write_all(line.as_bytes())
}

/// The journal's lines, oldest first, for tests and diagnosis.
pub fn read(dir: &Path) -> Vec<serde_json::Value> {
    std::fs::read_to_string(dir.join(FILE))
        .unwrap_or_default()
        .lines()
        .filter_map(|l| serde_json::from_str(l).ok())
        .collect()
}
