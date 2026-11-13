//! Reads SMF's deploy output into stages, warnings and errors (ADR 0003,
//! section 4). `Deploy.exe --colors` writes one `LEVEL<tab>message` line per
//! event, with DEBUG, INFO, WARN and ERROR levels. Only a few INFO lines are
//! stages; the rest are progress. A line this module doesn't recognise is
//! left alone: it stays in the raw log and never fails a deploy.

use std::collections::{BTreeMap, BTreeSet};

use vmm_core::backend::DeployLog;
use vmm_core::schema::{DeployStage, DeployWarning};

/// What the app itself writes into the log when Deploy.exe could not run.
pub const OWN_LINE: &str = "Vaexil Mod Manager: ";

const RAW_ENTITY: &str = "smf.raw_entity_file";
const OTHER: &str = "smf.warning";

/// INFO lines that are progress rather than a stage.
const NOT_STAGES: [&str; 3] = ["Initialising error reporting", "Discovering mod: ", "Analysing framework mod: "];

/// "Mod <name> replaces a base game entity file (<hash>) with a raw file."
fn raw_entity(message: &str) -> Option<(&str, &str)> {
    let rest = message.strip_prefix("Mod ")?;
    let at = rest.find(" replaces a base game entity file (")?;
    let name = &rest[..at];
    let after = &rest[at + " replaces a base game entity file (".len()..];
    let hash = &after[..after.find(')')?];
    after[hash.len()..].starts_with(") with a raw file").then_some((name, hash))
}

/// Warnings of one kind about one mod: the distinct keys seen, and examples.
type Group = (BTreeSet<String>, Vec<String>);

pub fn read(text: &str) -> DeployLog {
    let mut stages = Vec::new();
    let mut errors = Vec::new();
    let mut done = false;
    // (code, mod) -> (distinct keys, examples)
    let mut groups: BTreeMap<(&str, Option<String>), Group> = BTreeMap::new();
    let mut order: Vec<(&str, Option<String>)> = Vec::new();

    for line in text.lines() {
        if let Some(own) = line.strip_prefix(OWN_LINE) {
            errors.push(own.to_string());
            continue;
        }
        let Some((level, message)) = line.split_once('\t') else { continue };
        match level {
            "INFO" => {
                // A second tab marks a step inside one mod's deploy.
                if message.contains('\t') || NOT_STAGES.iter().any(|p| message.starts_with(p)) {
                    continue;
                }
                done |= message.starts_with("Done in ");
                let stage = match message.strip_prefix("Deploying ") {
                    Some(id) => DeployStage { name: "Deploying".into(), mod_id: Some(id.trim().to_string()) },
                    None => DeployStage { name: message.trim().to_string(), mod_id: None },
                };
                stages.push(stage);
            }
            "WARN" => {
                let (code, mod_name, key) = match raw_entity(message) {
                    Some((name, hash)) => (RAW_ENTITY, Some(name.to_string()), hash.to_string()),
                    None => (OTHER, None, message.to_string()),
                };
                let group = (code, mod_name);
                let entry = groups.entry(group.clone()).or_insert_with(|| {
                    order.push(group);
                    (BTreeSet::new(), Vec::new())
                });
                if entry.0.insert(key) && entry.1.len() < 5 {
                    entry.1.push(message.to_string());
                }
            }
            "ERROR" => errors.push(message.to_string()),
            _ => {}
        }
    }

    let warnings = order
        .into_iter()
        .map(|group| {
            let (keys, examples) = groups.remove(&group).expect("every group in order was recorded");
            let detail = if group.0 == RAW_ENTITY {
                "Replaces game entity files with whole raw files instead of entity.patch.json patches. It usually \
                 works, but it can clash with other mods that change the same entities, and it needs updating after \
                 game updates. Only the mod's author can change this."
                    .to_string()
            } else {
                "A warning from SMF this version doesn't recognise; see the examples or the log.".to_string()
            };
            DeployWarning { code: group.0.into(), mod_name: group.1, count: keys.len(), detail, examples }
        })
        .collect();
    DeployLog { stages, warnings, errors, done }
}

#[cfg(test)]
mod tests {
    use super::*;

    const LOG: &str = "\
INFO\tInitialising error reporting
INFO\tDiscovering mod contents
INFO\tDiscovering mod: Fixes
WARN\tMod Fixes 1.5 replaces a base game entity file (00B9217B4DB441A0) with a raw file. This can cause compatibility issues.
WARN\tMod Fixes 1.5 replaces a base game entity file (00B9217B4DB441A0) with a raw file. This can cause compatibility issues.
WARN\tMod Fixes 1.5 replaces a base game entity file (005CBB021B80DD36) with a raw file. This can cause compatibility issues.
WARN\tSomething new and odd
INFO\tInvalidating cache
INFO\tAnalysing framework mod: Fixes
INFO\tDeploying Some.Mod
DEBUG\tPreparing to apply patch C:\\x.json
INFO\tAccent Overhaul\tDeploying Japanese Hokkaido dialogue
INFO\tGenerating RPKGs
a line with no level at all
INFO\tDone in 15m
";

    #[test]
    fn reads_stages_and_counts_each_warning_once() {
        let r = read(LOG);
        let names: Vec<String> = r
            .stages
            .iter()
            .map(|s| match &s.mod_id {
                Some(m) => format!("{} {m}", s.name),
                None => s.name.clone(),
            })
            .collect();
        assert_eq!(
            names,
            ["Discovering mod contents", "Invalidating cache", "Deploying Some.Mod", "Generating RPKGs", "Done in 15m"]
        );
        assert!(r.done);
        assert_eq!(r.warnings.len(), 2);
        assert_eq!(
            (r.warnings[0].code.as_str(), r.warnings[0].mod_name.as_deref(), r.warnings[0].count),
            (RAW_ENTITY, Some("Fixes 1.5"), 2)
        );
        assert_eq!(r.warnings[0].examples.len(), 2);
        assert_eq!((r.warnings[1].code.as_str(), r.warnings[1].count), (OTHER, 1));
        assert!(r.errors.is_empty());
    }

    #[test]
    fn errors_and_the_apps_own_failure_are_errors() {
        let r = read("ERROR\tCould not find rpkg\nVaexil Mod Manager: Deploy.exe could not be started\n");
        assert_eq!(r.errors, ["Could not find rpkg", "Deploy.exe could not be started"]);
        assert!(!r.done);
    }
}
