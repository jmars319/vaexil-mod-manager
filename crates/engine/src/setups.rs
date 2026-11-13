//! Saved setups and the compatibility pass, kept in the app's own
//! `setups.json`. SMF never reads this file; switching to a setup or moving a
//! pass along writes SMF's `config.json` through the framework, with its
//! usual backup.
//!
//! A compatibility pass is the workflow for checking which mods still work
//! after a game update: a known-good baseline, then small batches, each tried
//! (deploy, check the log, play) and marked good, when it joins the baseline,
//! or bad, when the game goes back to the baseline. Saves are never touched.

use std::collections::BTreeMap;
use std::path::Path;

use serde_json::{Map, Value};
use vmm_core::backend::{ModEntry, Setup};
use vmm_core::error::{ErrorCode, VmmError};

use crate::store;

const FILE: &str = "setups.json";

/// Where the setups file is, in the data folder `dir`.
pub fn path(dir: &Path) -> std::path::PathBuf {
    dir.join(FILE)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NamedSetup {
    pub name: String,
    pub setup: Setup,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BatchStatus {
    Testing,
    Good,
    Bad,
}

impl BatchStatus {
    pub fn label(self) -> &'static str {
        match self {
            BatchStatus::Testing => "Testing",
            BatchStatus::Good => "Good",
            BatchStatus::Bad => "Bad",
        }
    }
    fn key(self) -> &'static str {
        match self {
            BatchStatus::Testing => "testing",
            BatchStatus::Good => "good",
            BatchStatus::Bad => "bad",
        }
    }
    fn from_key(k: &str) -> Option<Self> {
        [BatchStatus::Testing, BatchStatus::Good, BatchStatus::Bad].into_iter().find(|s| s.key() == k)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Batch {
    pub name: String,
    pub mods: Vec<String>,
    pub status: BatchStatus,
    pub notes: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Pass {
    /// The known-good mods, first to last.
    pub baseline: Vec<String>,
    pub batches: Vec<Batch>,
}

impl Pass {
    /// The batch being tried, if any. Only one at a time.
    pub fn active(&self) -> Option<usize> {
        self.batches.iter().position(|b| b.status == BatchStatus::Testing)
    }

    /// Mods that can go into the next batch: installed, readable, and not
    /// already in the baseline.
    pub fn candidates<'a>(&self, mods: &'a [ModEntry]) -> Vec<&'a ModEntry> {
        let mut out: Vec<&ModEntry> =
            mods.iter().filter(|m| m.problem.is_none() && !self.baseline.contains(&m.id)).collect();
        out.sort_by_key(|m| m.name.to_lowercase());
        out
    }

    pub fn begin(&mut self, name: &str, mods: Vec<String>) -> Result<(), VmmError> {
        if self.active().is_some() {
            return Err(VmmError::new(
                ErrorCode::PassState,
                "A batch is already being tried. Mark it good or bad first.",
            ));
        }
        if mods.is_empty() {
            return Err(VmmError::new(ErrorCode::PassState, "Pick at least one mod for the batch."));
        }
        if let Some(m) = mods.iter().find(|m| self.baseline.contains(m)) {
            return Err(VmmError::new(ErrorCode::PassState, format!("{m} is already in the baseline.")));
        }
        let name =
            if name.trim().is_empty() { format!("Batch {}", self.batches.len() + 1) } else { name.trim().to_string() };
        self.batches.push(Batch { name, mods, status: BatchStatus::Testing, notes: String::new() });
        Ok(())
    }

    /// Marks the batch being tried. `order` is the order it was tried in:
    /// when good, that order (baseline plus batch) becomes the new baseline.
    pub fn mark(&mut self, good: bool, order: &[String], notes: &str) -> Result<(), VmmError> {
        let i = self.active().ok_or_else(|| VmmError::new(ErrorCode::PassState, "No batch is being tried."))?;
        let batch = &mut self.batches[i];
        batch.status = if good { BatchStatus::Good } else { BatchStatus::Bad };
        batch.notes = notes.trim().to_string();
        if good {
            let keep: Vec<String> =
                order.iter().filter(|id| self.baseline.contains(id) || batch.mods.contains(id)).cloned().collect();
            self.baseline = keep;
        }
        Ok(())
    }

    /// The order the game should have now: the baseline, plus the batch being
    /// tried, each of its mods placed as late as its load rules allow.
    pub fn target_order(&self, mods: &[ModEntry]) -> Vec<String> {
        let mut order = self.baseline.clone();
        if let Some(i) = self.active() {
            for id in &self.batches[i].mods {
                if !order.contains(id) {
                    let at = vmm_core::order::insert_position(&order, id, mods);
                    order.insert(at, id.clone());
                }
            }
        }
        order
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Setups {
    pub saved: Vec<NamedSetup>,
    pub pass: Option<Pass>,
    rest: Map<String, Value>,
}

impl Setups {
    pub fn load(dir: &Path) -> Result<Setups, VmmError> {
        let mut map = store::load(dir, FILE)?;
        let saved = match map.remove("setups") {
            Some(Value::Array(list)) => list.iter().filter_map(read_setup).collect(),
            _ => Vec::new(),
        };
        let pass = map.remove("pass").as_ref().and_then(read_pass);
        Ok(Setups { saved, pass, rest: map })
    }

    pub fn save(&self, dir: &Path) -> Result<(), VmmError> {
        let mut map = self.rest.clone();
        map.insert("setups".into(), Value::Array(self.saved.iter().map(write_setup).collect()));
        match &self.pass {
            Some(p) => map.insert("pass".into(), write_pass(p)),
            None => map.remove("pass"),
        };
        store::save(dir, FILE, &map)
    }

    /// Saves `setup` under `name`, replacing a setup of the same name.
    pub fn put(&mut self, name: &str, setup: Setup) -> Result<(), VmmError> {
        let name = name.trim();
        if name.is_empty() {
            return Err(VmmError::new(ErrorCode::OptionInvalid, "Give the setup a name."));
        }
        match self.saved.iter_mut().find(|s| s.name.eq_ignore_ascii_case(name)) {
            Some(s) => s.setup = setup,
            None => self.saved.push(NamedSetup { name: name.to_string(), setup }),
        }
        Ok(())
    }

    /// The saved setup matching the mods that are on now, if any.
    pub fn matching(&self, now: &Setup) -> Option<&NamedSetup> {
        self.saved.iter().find(|s| s.setup.order == now.order)
    }
}

fn read_setup(v: &Value) -> Option<NamedSetup> {
    let name = v.get("name")?.as_str()?.to_string();
    let order = store::strings(v.get("order"));
    let options: BTreeMap<String, Vec<String>> = v
        .get("options")
        .and_then(Value::as_object)
        .map(|o| o.iter().map(|(k, v)| (k.clone(), store::strings(Some(v)))).collect())
        .unwrap_or_default();
    Some(NamedSetup { name, setup: Setup { order, options } })
}

fn write_setup(s: &NamedSetup) -> Value {
    let mut m = Map::new();
    m.insert("name".into(), Value::String(s.name.clone()));
    m.insert("order".into(), store::to_strings(&s.setup.order));
    m.insert(
        "options".into(),
        Value::Object(s.setup.options.iter().map(|(k, v)| (k.clone(), store::to_strings(v))).collect()),
    );
    Value::Object(m)
}

fn read_pass(v: &Value) -> Option<Pass> {
    let batches = v
        .get("batches")
        .and_then(Value::as_array)
        .map(|list| {
            list.iter()
                .filter_map(|b| {
                    Some(Batch {
                        name: b.get("name")?.as_str()?.to_string(),
                        mods: store::strings(b.get("mods")),
                        status: BatchStatus::from_key(b.get("status")?.as_str()?)?,
                        notes: b.get("notes").and_then(Value::as_str).unwrap_or_default().to_string(),
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    Some(Pass { baseline: store::strings(v.get("baseline")), batches })
}

fn write_pass(p: &Pass) -> Value {
    let batches = p
        .batches
        .iter()
        .map(|b| {
            let mut m = Map::new();
            m.insert("name".into(), Value::String(b.name.clone()));
            m.insert("mods".into(), store::to_strings(&b.mods));
            m.insert("status".into(), Value::String(b.status.key().into()));
            m.insert("notes".into(), Value::String(b.notes.clone()));
            Value::Object(m)
        })
        .collect();
    let mut m = Map::new();
    m.insert("baseline".into(), store::to_strings(&p.baseline));
    m.insert("batches".into(), Value::Array(batches));
    Value::Object(m)
}

#[cfg(test)]
mod tests {
    use vmm_core::backend::{OrderRule, Relation};

    use super::*;

    fn m(id: &str, rules: &[(Relation, &str)]) -> ModEntry {
        ModEntry {
            id: id.into(),
            name: id.into(),
            rules: rules.iter().map(|(r, o)| OrderRule { relation: *r, other: o.to_string() }).collect(),
            ..ModEntry::default()
        }
    }

    fn ids(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn a_pass_grows_the_baseline_with_good_batches_only() {
        let mods = vec![m("Base", &[]), m("A", &[]), m("B", &[(Relation::Before, "Base")]), m("C", &[])];
        let mut pass = Pass { baseline: ids(&["Base"]), batches: vec![] };
        assert_eq!(pass.candidates(&mods).len(), 3);

        pass.begin("", ids(&["A", "B"])).unwrap();
        assert_eq!(pass.batches[0].name, "Batch 1");
        assert!(pass.begin("Again", ids(&["C"])).unwrap_err().message.contains("already being tried"));
        let tried = pass.target_order(&mods);
        assert_eq!(tried, ids(&["B", "Base", "A"]));
        pass.mark(true, &tried, "  stable for an hour ").unwrap();
        assert_eq!(pass.baseline, ids(&["B", "Base", "A"]));
        assert_eq!(pass.batches[0].notes, "stable for an hour");

        pass.begin("Risky", ids(&["C"])).unwrap();
        assert_eq!(pass.target_order(&mods), ids(&["B", "Base", "A", "C"]));
        let tried = pass.target_order(&mods);
        pass.mark(false, &tried, "crash at the main menu").unwrap();
        assert_eq!(pass.baseline, ids(&["B", "Base", "A"]));
        assert_eq!(pass.target_order(&mods), pass.baseline);
        assert_eq!(pass.batches[1].status, BatchStatus::Bad);

        assert!(pass.begin("", vec![]).is_err());
        assert!(pass.begin("", ids(&["A"])).unwrap_err().message.contains("baseline"));
        assert!(pass.mark(true, &[], "").is_err());
    }

    #[test]
    fn setups_and_a_pass_survive_a_save() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join(FILE), r#"{ "future": 1 }"#).unwrap();
        let mut s = Setups::load(tmp.path()).unwrap();
        let setup = Setup { order: ids(&["A", "B"]), options: BTreeMap::from([("A".into(), ids(&["X"]))]) };
        s.put("Everyday", setup.clone()).unwrap();
        s.put("everyday", setup.clone()).unwrap();
        assert_eq!(s.saved.len(), 1);
        assert!(s.put(" ", setup.clone()).is_err());
        let mut pass = Pass { baseline: ids(&["A"]), batches: vec![] };
        pass.begin("One", ids(&["B"])).unwrap();
        s.pass = Some(pass);
        s.save(tmp.path()).unwrap();

        let back = Setups::load(tmp.path()).unwrap();
        assert_eq!(back, s);
        assert_eq!(back.matching(&setup).map(|n| n.name.as_str()), Some("Everyday"));
        assert!(std::fs::read_to_string(tmp.path().join(FILE)).unwrap().contains("future"));
    }
}
