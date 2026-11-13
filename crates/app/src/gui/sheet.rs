//! The edit sheet for one mod: its type and its options. Plain Rust, so it is
//! tested without a window. Nothing is saved until Save.

use vmm_core::framework::{ModEntry, ModKind};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EditSheet {
    pub mod_id: String,
    pub title: String,
    pub subtitle: String,
    /// "Automatic: <kind>" first, then every kind.
    pub kind_labels: Vec<String>,
    /// 0 is automatic.
    pub kind_index: usize,
    pub checks: Vec<Check>,
    pub selects: Vec<Select>,
    original_kind_index: usize,
    original_chosen: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Check {
    pub value: String,
    pub label: String,
    /// The tooltip, and what the option needs when it can't be chosen yet.
    pub detail: String,
    pub checked: bool,
    pub enabled: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Select {
    pub group: String,
    pub labels: Vec<String>,
    pub values: Vec<String>,
    /// Whether each option's required mods are on.
    pub usable: Vec<bool>,
    pub index: usize,
}

impl Select {
    /// False when every option needs a mod that is off: the group can't be
    /// used, so it is shown but not changeable, and not saved.
    pub fn enabled(&self) -> bool {
        self.usable.iter().any(|u| *u)
    }
}

fn needs_note(needs: &[String]) -> String {
    if needs.is_empty() {
        String::new()
    } else {
        format!("Needs {} turned on first. SMF leaves it out until then.", needs.join(", "))
    }
}

impl EditSheet {
    /// `detected` is the kind worked out from the mod's files; `chosen` the
    /// user's override, if any.
    pub fn new(m: &ModEntry, detected: ModKind, chosen: Option<ModKind>) -> EditSheet {
        let mut kind_labels = vec![format!("Automatic: {}", detected.label())];
        kind_labels.extend(ModKind::ALL.iter().map(|k| k.label().to_string()));
        let kind_index = chosen.and_then(|k| ModKind::ALL.iter().position(|a| *a == k)).map_or(0, |i| i + 1);

        let mut checks = Vec::new();
        let mut selects: Vec<Select> = Vec::new();
        for o in &m.options {
            match &o.group {
                None => checks.push(Check {
                    value: o.value.clone(),
                    label: o.label.clone(),
                    detail: [o.tooltip.clone().unwrap_or_default(), needs_note(&o.needs)]
                        .into_iter()
                        .filter(|s| !s.is_empty())
                        .collect::<Vec<_>>()
                        .join(" "),
                    checked: o.chosen,
                    enabled: o.needs.is_empty(),
                }),
                Some(group) => {
                    let label = if o.needs.is_empty() {
                        o.label.clone()
                    } else {
                        format!("{} (needs {} on)", o.label, o.needs.join(", "))
                    };
                    match selects.iter_mut().find(|s| s.group == *group) {
                        Some(s) => {
                            if o.chosen {
                                s.index = s.values.len();
                            }
                            s.labels.push(label);
                            s.values.push(o.value.clone());
                            s.usable.push(o.needs.is_empty());
                        }
                        None => selects.push(Select {
                            group: group.clone(),
                            labels: vec![label],
                            values: vec![o.value.clone()],
                            usable: vec![o.needs.is_empty()],
                            index: 0,
                        }),
                    }
                }
            }
        }
        let mut sheet = EditSheet {
            mod_id: m.id.clone(),
            title: m.name.clone(),
            subtitle: String::new(),
            kind_labels,
            kind_index,
            checks,
            selects,
            original_kind_index: kind_index,
            original_chosen: Vec::new(),
        };
        sheet.subtitle = if m.options.is_empty() {
            "This mod has no options. Its type only groups the Mods view and is kept by Vaexil Mod Manager.".into()
        } else {
            "Options are saved to SMF's config.json, after keeping a copy, and reach the game at the next deploy. \
             The type only groups the Mods view."
                .into()
        };
        sheet.original_chosen = sheet.chosen();
        sheet
    }

    /// The chosen options, as the framework stores them. An option whose
    /// required mods are off never counts: SMF would leave it out anyway, and
    /// counting it would make the whole save fail.
    pub fn chosen(&self) -> Vec<String> {
        let checks = self.checks.iter().filter(|c| c.checked && c.enabled).map(|c| c.value.clone());
        let selects = self
            .selects
            .iter()
            .filter(|s| s.usable.get(s.index).copied().unwrap_or(false))
            .filter_map(|s| s.values.get(s.index).cloned());
        checks.chain(selects).collect()
    }

    /// `None` is automatic.
    pub fn kind(&self) -> Option<ModKind> {
        self.kind_index.checked_sub(1).and_then(|i| ModKind::ALL.get(i).copied())
    }

    pub fn options_changed(&self) -> bool {
        let mut a = self.chosen();
        let mut b = self.original_chosen.clone();
        a.sort();
        b.sort();
        a != b
    }

    pub fn kind_changed(&self) -> bool {
        self.kind_index != self.original_kind_index
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use vmm_core::framework::ModOption;

    use super::*;

    fn opt(value: &str, group: Option<&str>, chosen: bool, needs: &[&str]) -> ModOption {
        ModOption {
            value: value.into(),
            label: value.rsplit(':').next().unwrap().into(),
            group: group.map(Into::into),
            tooltip: (value == "Darker").then(|| "Darker suits".into()),
            chosen,
            needs: needs.iter().map(|s| s.to_string()).collect(),
        }
    }

    fn entry(options: Vec<ModOption>) -> ModEntry {
        ModEntry {
            id: "Id.M".into(),
            name: "Mod".into(),
            version: None,
            authors: vec![],
            folder: PathBuf::from("M"),
            badge: None,
            kind: ModKind::Gameplay,
            load_position: Some(1),
            options,
            ..ModEntry::default()
        }
    }

    fn sample() -> EditSheet {
        let m = entry(vec![
            opt("Darker", None, true, &[]),
            opt("With A", None, false, &["Mod.A"]),
            opt("Hands:Bare", Some("Hands"), false, &[]),
            opt("Hands:Gloves", Some("Hands"), true, &[]),
        ]);
        EditSheet::new(&m, ModKind::Gameplay, None)
    }

    #[test]
    fn lays_out_checkboxes_and_one_choice_per_group() {
        let s = sample();
        assert_eq!(s.kind_labels[0], "Automatic: Gameplay");
        assert_eq!(s.kind_labels.len(), 1 + ModKind::ALL.len());
        assert_eq!(s.kind_index, 0);
        assert_eq!(s.checks.len(), 2);
        assert_eq!(s.checks[0].detail, "Darker suits");
        assert!(!s.checks[1].enabled);
        assert!(s.checks[1].detail.contains("Needs Mod.A"));
        assert_eq!(s.selects.len(), 1);
        assert_eq!(s.selects[0].labels, ["Bare", "Gloves"]);
        assert_eq!(s.selects[0].index, 1);
        assert_eq!(s.chosen(), ["Darker", "Hands:Gloves"]);
        assert!(!s.options_changed() && !s.kind_changed());
    }

    #[test]
    fn tracks_what_changed() {
        let mut s = sample();
        s.checks[0].checked = false;
        s.selects[0].index = 0;
        assert_eq!(s.chosen(), ["Hands:Bare"]);
        assert!(s.options_changed());
        s.kind_index = 1 + ModKind::ALL.iter().position(|k| *k == ModKind::Sound).unwrap();
        assert_eq!(s.kind(), Some(ModKind::Sound));
        assert!(s.kind_changed());
    }

    #[test]
    fn options_whose_mods_are_off_never_block_a_save() {
        let m = entry(vec![
            opt("Darker", None, false, &[]),
            opt("With A", None, true, &["Mod.A"]),
            opt("Mood:Calm", Some("Mood"), false, &["Mod.A"]),
            opt("Mood:Angry", Some("Mood"), false, &["Mod.A"]),
        ]);
        let mut s = EditSheet::new(&m, ModKind::Gameplay, None);
        assert!(!s.checks[1].enabled);
        assert!(!s.selects[0].enabled());
        assert!(s.chosen().is_empty());
        s.checks[0].checked = true;
        assert_eq!(s.chosen(), ["Darker"]);
    }

    #[test]
    fn opens_on_the_users_chosen_type() {
        let s = EditSheet::new(&entry(vec![]), ModKind::Other, Some(ModKind::Ui));
        assert_eq!(s.kind(), Some(ModKind::Ui));
        assert!(s.subtitle.contains("no options"));
    }
}
