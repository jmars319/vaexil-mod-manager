//! Load-order reasoning that does not depend on any one framework: which of
//! the mods' own ordering rules the current order breaks, a stable sort that
//! satisfies them, where a newly enabled mod belongs, and which mod's files
//! win when two mods replace the same file.
//!
//! An order lists mod ids first to last. The later mod wins, as in SMF.

use std::collections::{BTreeMap, HashMap, HashSet};

use crate::backend::{ModEntry, Relation};

/// A rule the order does not keep: `mod_id` should load `relation` `other`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BrokenRule {
    pub mod_id: String,
    pub relation: Relation,
    pub other: String,
}

/// Every pair "x must come before y" among the ids in `order`, from both
/// sides' rules.
fn edges(order: &[String], mods: &[ModEntry]) -> HashSet<(String, String)> {
    let present: HashSet<&str> = order.iter().map(String::as_str).collect();
    let mut out = HashSet::new();
    for m in mods.iter().filter(|m| present.contains(m.id.as_str())) {
        for r in &m.rules {
            if r.other == m.id || !present.contains(r.other.as_str()) {
                continue;
            }
            match r.relation {
                Relation::Before => out.insert((m.id.clone(), r.other.clone())),
                Relation::After => out.insert((r.other.clone(), m.id.clone())),
            };
        }
    }
    out
}

/// The rules the order breaks, in the order the mods appear.
pub fn broken_rules(order: &[String], mods: &[ModEntry]) -> Vec<BrokenRule> {
    let index: HashMap<&str, usize> = order.iter().enumerate().map(|(i, id)| (id.as_str(), i)).collect();
    let mut out = Vec::new();
    for id in order {
        let Some(m) = mods.iter().find(|m| m.id == *id) else { continue };
        for r in &m.rules {
            let (Some(&a), Some(&b)) = (index.get(m.id.as_str()), index.get(r.other.as_str())) else { continue };
            let kept = match r.relation {
                Relation::Before => a < b,
                Relation::After => a > b,
            };
            if !kept {
                out.push(BrokenRule { mod_id: m.id.clone(), relation: r.relation, other: r.other.clone() });
            }
        }
    }
    out
}

/// Reorders `order` so every rule holds, moving as little as possible: each
/// mod keeps its place, except that a mod that must load before it is moved
/// up to just ahead of it. Rules that contradict each other make this fail
/// with the ids caught in them.
pub fn auto_sort(order: &[String], mods: &[ModEntry]) -> Result<Vec<String>, Vec<String>> {
    let edges = edges(order, mods);
    let mut out: Vec<String> = Vec::with_capacity(order.len());
    let mut stack: Vec<String> = Vec::new();
    fn visit(
        id: &String,
        order: &[String],
        edges: &HashSet<(String, String)>,
        out: &mut Vec<String>,
        stack: &mut Vec<String>,
    ) -> Result<(), Vec<String>> {
        if out.contains(id) {
            return Ok(());
        }
        if let Some(i) = stack.iter().position(|s| s == id) {
            return Err(stack[i..].to_vec());
        }
        stack.push(id.clone());
        // Predecessors in their current order, so the result is stable.
        for p in order.iter().filter(|p| edges.contains(&((*p).clone(), id.clone()))) {
            visit(p, order, edges, out, stack)?;
        }
        stack.pop();
        out.push(id.clone());
        Ok(())
    }
    for id in order {
        visit(id, order, &edges, &mut out, &mut stack)?;
    }
    Ok(out)
}

/// Where `new_id` goes when it is turned on: as late as its rules allow, so
/// that, like SMF's own Mod Manager, a new mod usually wins. When its rules
/// can't all be kept, it goes last and the view shows the broken rule.
pub fn insert_position(order: &[String], new_id: &str, mods: &[ModEntry]) -> usize {
    let mut with_new = order.to_vec();
    with_new.push(new_id.to_string());
    let edges = edges(&with_new, mods);
    let mut lo = 0;
    let mut hi = order.len();
    for (i, id) in order.iter().enumerate() {
        if edges.contains(&(id.clone(), new_id.to_string())) {
            lo = lo.max(i + 1);
        }
        if edges.contains(&(new_id.to_string(), id.clone())) {
            hi = hi.min(i);
        }
    }
    if lo <= hi { hi } else { order.len() }
}

/// For each mod in `order`, the later mods that replace one of its files
/// wholesale, so its version of that file is not used.
pub fn overridden_by(order: &[String], mods: &[ModEntry]) -> BTreeMap<String, Vec<String>> {
    let mut owners: HashMap<&str, Vec<&str>> = HashMap::new();
    for id in order {
        let Some(m) = mods.iter().find(|m| m.id == *id) else { continue };
        for f in &m.replaces {
            let list = owners.entry(f.as_str()).or_default();
            if !list.contains(&m.id.as_str()) {
                list.push(m.id.as_str());
            }
        }
    }
    let mut out: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for list in owners.values() {
        for (i, loser) in list.iter().enumerate() {
            for winner in &list[i + 1..] {
                let entry = out.entry(loser.to_string()).or_default();
                if !entry.iter().any(|w| w == winner) {
                    entry.push(winner.to_string());
                }
            }
        }
    }
    let position = |id: &str| order.iter().position(|o| o == id).unwrap_or(usize::MAX);
    for winners in out.values_mut() {
        winners.sort_by_key(|w| position(w));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::OrderRule;

    fn m(id: &str, rules: &[(Relation, &str)], replaces: &[&str]) -> ModEntry {
        ModEntry {
            id: id.into(),
            name: id.into(),
            rules: rules.iter().map(|(r, o)| OrderRule { relation: *r, other: o.to_string() }).collect(),
            replaces: replaces.iter().map(|s| s.to_string()).collect(),
            ..ModEntry::default()
        }
    }

    fn ids(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn finds_broken_rules_from_either_side() {
        let mods = vec![
            m("Patches", &[], &[]),
            m("Renaming", &[(Relation::After, "Patches")], &[]),
            m("Lighting", &[(Relation::Before, "Suits")], &[]),
            m("Suits", &[], &[]),
        ];
        assert!(broken_rules(&ids(&["Patches", "Renaming", "Lighting", "Suits"]), &mods).is_empty());
        let broken = broken_rules(&ids(&["Renaming", "Suits", "Patches", "Lighting"]), &mods);
        assert_eq!(
            broken,
            [
                BrokenRule { mod_id: "Renaming".into(), relation: Relation::After, other: "Patches".into() },
                BrokenRule { mod_id: "Lighting".into(), relation: Relation::Before, other: "Suits".into() },
            ]
        );
        // A rule about a mod that is off is not broken.
        assert!(broken_rules(&ids(&["Renaming"]), &mods).is_empty());
    }

    #[test]
    fn auto_sort_keeps_the_users_order_where_no_rule_decides() {
        let mods = vec![
            m("A", &[], &[]),
            m("B", &[(Relation::After, "D")], &[]),
            m("C", &[], &[]),
            m("D", &[], &[]),
            m("E", &[(Relation::Before, "A")], &[]),
        ];
        let sorted = auto_sort(&ids(&["A", "B", "C", "D", "E"]), &mods).unwrap();
        assert_eq!(sorted, ids(&["E", "A", "D", "B", "C"]));
        assert!(broken_rules(&sorted, &mods).is_empty());
        // Already sorted: unchanged.
        assert_eq!(auto_sort(&sorted, &mods).unwrap(), sorted);
    }

    #[test]
    fn auto_sort_names_rules_that_contradict() {
        let mods = vec![
            m("A", &[(Relation::Before, "B")], &[]),
            m("B", &[(Relation::Before, "C")], &[]),
            m("C", &[(Relation::Before, "A")], &[]),
            m("Free", &[], &[]),
        ];
        let mut caught = auto_sort(&ids(&["Free", "A", "B", "C"]), &mods).unwrap_err();
        caught.sort();
        assert_eq!(caught, ids(&["A", "B", "C"]));
    }

    #[test]
    fn a_new_mod_goes_as_late_as_its_rules_allow() {
        let mods = vec![
            m("A", &[], &[]),
            m("B", &[], &[]),
            m("C", &[(Relation::After, "New")], &[]),
            m("New", &[(Relation::After, "A")], &[]),
        ];
        let order = ids(&["A", "B", "C"]);
        assert_eq!(insert_position(&order, "New", &mods), 2);
        assert_eq!(insert_position(&order, "Plain", &mods), 3);
        // Impossible: must be after C but before A. It goes last.
        let mods =
            vec![m("A", &[], &[]), m("C", &[], &[]), m("New", &[(Relation::After, "C"), (Relation::Before, "A")], &[])];
        assert_eq!(insert_position(&ids(&["A", "C"]), "New", &mods), 2);
    }

    #[test]
    fn the_later_mod_wins_a_shared_file() {
        let mods = vec![
            m("Fixes", &[], &["00AA.TEMP", "00BB.PRIM"]),
            m("Suits", &[], &["00AA.TEMP"]),
            m("Glasses", &[], &["00AA.TEMP", "00CC.TEXT"]),
            m("Off", &[], &["00BB.PRIM"]),
        ];
        let over = overridden_by(&ids(&["Fixes", "Suits", "Glasses"]), &mods);
        assert_eq!(over.get("Fixes"), Some(&ids(&["Suits", "Glasses"])));
        assert_eq!(over.get("Suits"), Some(&ids(&["Glasses"])));
        assert_eq!(over.get("Glasses"), None);
    }
}
