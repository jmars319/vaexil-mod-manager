//! The Load order view, and the load-order warnings both views show. Plain
//! Rust, so it is tested without a window.

use std::collections::BTreeMap;

use vmm_core::framework::{ModEntry, ModKind, Relation};
use vmm_core::order;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct OrderRow {
    pub id: String,
    pub name: String,
    pub position: i32,
    /// Type, and "RPKG" for RPKG-only mods.
    pub detail: String,
    /// Broken rules and overrides, in plain words.
    pub notice: String,
    pub can_up: bool,
    pub can_down: bool,
}

/// The enabled mods' ids, first to last, as the framework's settings hold them.
pub fn current_order(mods: &[ModEntry]) -> Vec<String> {
    let mut on: Vec<&ModEntry> = mods.iter().filter(|m| m.load_position.is_some()).collect();
    on.sort_by_key(|m| m.load_position);
    on.into_iter().map(|m| m.id.clone()).collect()
}

fn name_of<'a>(mods: &'a [ModEntry], id: &'a str) -> &'a str {
    mods.iter().find(|m| m.id == id).map_or(id, |m| m.name.as_str())
}

/// For each mod in `order`, what is wrong with its place, in plain words:
/// rules it or another mod has about it that the order breaks, and the later
/// mods whose files replace its own.
pub fn notices(order: &[String], mods: &[ModEntry]) -> BTreeMap<String, String> {
    let mut out: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for b in order::broken_rules(order, mods) {
        let (me, other) = (name_of(mods, &b.mod_id), name_of(mods, &b.other));
        let (mine, theirs) = match b.relation {
            Relation::Before => (format!("Should load before {other}"), format!("{me} should load before it")),
            Relation::After => (format!("Should load after {other}"), format!("{me} should load after it")),
        };
        out.entry(b.mod_id.clone()).or_default().push(mine);
        out.entry(b.other.clone()).or_default().push(theirs);
    }
    for (id, winners) in order::overridden_by(order, mods) {
        let names: Vec<&str> = winners.iter().map(|w| name_of(mods, w)).collect();
        out.entry(id).or_default().push(format!("Some files are replaced by {}", names.join(", ")));
    }
    out.into_iter().map(|(id, parts)| (id, parts.join(". "))).collect()
}

/// The rows of the Load order view for `order`, which may be an unsaved draft.
pub fn rows(order: &[String], mods: &[ModEntry], kinds: &BTreeMap<String, ModKind>) -> Vec<OrderRow> {
    let notices = notices(order, mods);
    let last = order.len().saturating_sub(1);
    order
        .iter()
        .enumerate()
        .map(|(i, id)| {
            let m = mods.iter().find(|m| m.id == *id);
            let kind = m.map(|m| kinds.get(&m.id).copied().unwrap_or(m.kind)).unwrap_or_default();
            let mut detail = vec![kind.label().to_string()];
            if m.and_then(|m| m.badge.as_deref()) == Some("RPKG") {
                detail.push("RPKG mod, always deployed before framework mods".into());
            }
            OrderRow {
                id: id.clone(),
                name: name_of(mods, id).to_string(),
                position: i as i32 + 1,
                detail: detail.join(" · "),
                notice: notices.get(id).cloned().unwrap_or_default(),
                can_up: i > 0,
                can_down: i < last,
            }
        })
        .collect()
}

/// The line above the list: how many mods, how many broken rules, and
/// whether there are unsaved changes.
pub fn summary(order: &[String], mods: &[ModEntry], dirty: bool) -> String {
    let broken = order::broken_rules(order, mods).len();
    let mut parts = vec![match order.len() {
        1 => "1 mod on".to_string(),
        n => format!("{n} mods on"),
    }];
    match broken {
        0 => parts.push("every load rule kept".into()),
        1 => parts.push("1 load rule broken".into()),
        n => parts.push(format!("{n} load rules broken")),
    }
    if dirty {
        parts.push("unsaved changes".into());
    }
    parts.join(" · ")
}

/// Moves the mod at `from` by `delta` places, clamped to the list.
pub fn moved(order: &[String], from: usize, delta: i32) -> Vec<String> {
    let mut out = order.to_vec();
    if from >= out.len() {
        return out;
    }
    let to = (from as i64 + delta as i64).clamp(0, out.len() as i64 - 1) as usize;
    let id = out.remove(from);
    out.insert(to, id);
    out
}

#[cfg(test)]
mod tests {
    use vmm_core::framework::OrderRule;

    use super::*;

    fn m(id: &str, pos: Option<usize>, rules: &[(Relation, &str)], replaces: &[&str]) -> ModEntry {
        ModEntry {
            id: id.into(),
            name: format!("{id} mod"),
            load_position: pos,
            rules: rules.iter().map(|(r, o)| OrderRule { relation: *r, other: o.to_string() }).collect(),
            replaces: replaces.iter().map(|s| s.to_string()).collect(),
            ..ModEntry::default()
        }
    }

    fn sample() -> Vec<ModEntry> {
        vec![
            m("B", Some(1), &[(Relation::After, "A")], &["X.TEMP"]),
            m("A", Some(2), &[], &["X.TEMP"]),
            m("C", Some(3), &[], &[]),
            m("Off", None, &[], &["X.TEMP"]),
        ]
    }

    #[test]
    fn words_broken_rules_and_overrides_on_both_mods() {
        let mods = sample();
        let order = current_order(&mods);
        assert_eq!(order, ["B", "A", "C"]);
        let n = notices(&order, &mods);
        assert_eq!(n["B"], "Should load after A mod. Some files are replaced by A mod");
        assert_eq!(n["A"], "B mod should load after it");
        assert!(!n.contains_key("C"));
        assert_eq!(summary(&order, &mods, true), "3 mods on · 1 load rule broken · unsaved changes");
    }

    #[test]
    fn rows_know_their_neighbours_and_moves_clamp() {
        let mods = sample();
        let order = current_order(&mods);
        let r = rows(&order, &mods, &BTreeMap::new());
        assert_eq!((r[0].can_up, r[0].can_down, r[2].can_down), (false, true, false));
        assert_eq!(r[1].position, 2);
        assert_eq!(r[0].detail, "Other");
        assert_eq!(moved(&order, 0, 1), ["A", "B", "C"]);
        assert_eq!(moved(&order, 2, -5), ["C", "B", "A"]);
        assert_eq!(moved(&order, 1, 9), ["B", "C", "A"]);
        let fixed = moved(&order, 0, 1);
        assert_eq!(summary(&fixed, &mods, false), "3 mods on · every load rule kept");
    }
}
