//! Turns what the core found into the words the window shows. Plain Rust, so
//! it is tested without a window.

use vmm_core::framework::{Framework, Inspection, ModEntry, ModKind, Status, Version};
use vmm_core::game::GameInstall;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Screen {
    /// The mod list.
    Mods,
    /// A full-window message instead of the list.
    Message,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct View {
    pub screen: Screen,
    pub game_detail: String,
    pub chip: String,
    pub chip_warn: bool,
    pub heading: String,
    pub message: String,
    pub summary: String,
    /// Group headings and mods, in the order the list shows them.
    pub rows: Vec<Row>,
    pub notes: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Row {
    /// Set on a group heading row, such as "Sound"; the other fields are empty.
    pub heading: String,
    /// The id the framework's load order uses.
    pub id: String,
    pub name: String,
    pub detail: String,
    /// 1-based load position, or 0 when the mod is off.
    pub position: i32,
    pub badge: String,
    pub problem: String,
    /// Whether the switch can be used. A mod whose manifest cannot be read
    /// cannot be turned on, but can be turned off.
    pub can_toggle: bool,
}

pub fn build(installs: &[GameInstall], framework: &dyn Framework) -> View {
    let Some(game) = installs.first() else {
        return message(
            "Not found",
            true,
            "",
            "HITMAN isn't installed on this PC",
            "Vaexil Mod Manager looked for HITMAN World of Assassination in your Steam libraries, Epic Games \
             and the Microsoft Store's XboxGames folders. This window checks again every few seconds.",
        );
    };
    let mut notes: Vec<String> = installs[1..]
        .iter()
        .map(|g| {
            format!(
                "HITMAN is also installed through {} at {}. This version manages the first one only.",
                g.store.label(),
                g.root.display()
            )
        })
        .collect();
    let game_detail = format!("{} · {}", game.store.label(), game.root.display());
    let inspection: Inspection = framework.inspect(game);
    match inspection.status {
        Status::NotInstalled => message(
            "Not installed",
            true,
            &game_detail,
            &format!("{} isn't installed", framework.name()),
            &format!(
                "HITMAN is at {}, but there is no {} folder inside it. Install the official release from \
                 github.com/atampy25/simple-mod-framework into a folder in the game folder, such as \
                 \"Simple Mod Framework\". This window checks again every few seconds.",
                game.root.display(),
                framework.name()
            ),
        ),
        Status::Unreadable { folder, reason } => message(
            "Can't read",
            true,
            &format!("{} · {}", game.store.label(), folder.display()),
            &format!("{}'s settings can't be read", framework.name()),
            &reason,
        ),
        Status::Installed { folder, version } => {
            notes.extend(inspection.notes);
            let enabled = inspection.mods.iter().filter(|m| m.load_position.is_some()).count();
            let summary = match inspection.mods.len() {
                0 => "No mods in SMF's Mods folder yet".to_string(),
                1 => format!("1 mod · {enabled} on"),
                n => format!("{n} mods · {enabled} on"),
            };
            View {
                screen: Screen::Mods,
                // The SMF folder sits inside the game folder, so it says both.
                game_detail: format!("{} · {}", game.store.label(), folder.display()),
                chip_warn: matches!(version, Version::Unknown { .. }),
                chip: format!("SMF {}", version.label()),
                heading: String::new(),
                message: String::new(),
                summary,
                rows: grouped(&inspection.mods),
                notes,
            }
        }
    }
}

/// One heading per kind that has mods, in [`ModKind::ALL`] order, each
/// followed by its mods by name. The grouping is information only; the load
/// position on each row is what SMF uses.
fn grouped(mods: &[ModEntry]) -> Vec<Row> {
    let mut rows = Vec::new();
    for kind in ModKind::ALL {
        let mut group: Vec<&ModEntry> = mods.iter().filter(|m| m.kind == kind).collect();
        if group.is_empty() {
            continue;
        }
        group.sort_by_key(|m| m.name.to_lowercase());
        rows.push(Row { heading: kind.label().to_string(), ..Row::default() });
        rows.extend(group.into_iter().map(row));
    }
    rows
}

fn message(chip: &str, chip_warn: bool, game_detail: &str, heading: &str, message: &str) -> View {
    View {
        screen: Screen::Message,
        game_detail: game_detail.to_string(),
        chip: chip.to_string(),
        chip_warn,
        heading: heading.to_string(),
        message: message.to_string(),
        summary: String::new(),
        rows: Vec::new(),
        notes: Vec::new(),
    }
}

fn row(m: &ModEntry) -> Row {
    let mut parts = Vec::new();
    if let Some(v) = &m.version {
        parts.push(v.clone());
    }
    if !m.authors.is_empty() {
        parts.push(m.authors.join(", "));
    }
    if m.badge.as_deref() == Some("RPKG") {
        parts.push("RPKG mod, no manifest".to_string());
    }
    Row {
        heading: String::new(),
        id: m.id.clone(),
        name: m.name.clone(),
        detail: parts.join(" · "),
        position: m.load_position.map_or(0, |p| p as i32),
        badge: m.badge.clone().unwrap_or_default(),
        problem: m.problem.clone().unwrap_or_default(),
        can_toggle: m.problem.is_none() || m.load_position.is_some(),
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use vmm_core::game::Store;

    use super::*;

    struct Fake(Inspection);

    impl Framework for Fake {
        fn name(&self) -> &'static str {
            "Simple Mod Framework"
        }
        fn inspect(&self, _: &GameInstall) -> Inspection {
            self.0.clone()
        }
        fn set_enabled(&self, _: &GameInstall, _: &str, _: bool) -> Result<bool, String> {
            unreachable!("the view never writes")
        }
    }

    fn game(store: Store, root: &str) -> GameInstall {
        GameInstall { store, root: PathBuf::from(root) }
    }

    fn entry(name: &str, kind: ModKind, position: Option<usize>) -> ModEntry {
        ModEntry {
            id: format!("Id.{name}"),
            name: name.into(),
            version: Some("1.0.0".into()),
            authors: vec!["A".into(), "B".into()],
            folder: PathBuf::from(name),
            badge: None,
            kind,
            load_position: position,
            problem: None,
        }
    }

    fn installed(mods: Vec<ModEntry>) -> Fake {
        Fake(Inspection {
            status: Status::Installed { folder: PathBuf::from(r"C:\G\SMF"), version: Version::Known("2.33.45".into()) },
            mods,
            notes: vec![],
        })
    }

    #[test]
    fn no_game_shows_a_message() {
        let view = build(&[], &Fake(Inspection { status: Status::NotInstalled, mods: vec![], notes: vec![] }));
        assert_eq!(view.screen, Screen::Message);
        assert!(view.heading.contains("HITMAN isn't installed"));
    }

    #[test]
    fn missing_framework_names_the_game_folder() {
        let fw = Fake(Inspection { status: Status::NotInstalled, mods: vec![], notes: vec![] });
        let view = build(&[game(Store::Steam, r"C:\Games\HITMAN 3")], &fw);
        assert_eq!(view.screen, Screen::Message);
        assert!(view.message.contains(r"C:\Games\HITMAN 3"));
        assert_eq!(view.game_detail, r"Steam · C:\Games\HITMAN 3");
    }

    #[test]
    fn installed_framework_lists_mods_and_counts_enabled_ones() {
        let mut fw = installed(vec![entry("One", ModKind::Sound, Some(1)), entry("Two", ModKind::Sound, None)]);
        fw.0.notes = vec!["a note".into()];
        let view = build(&[game(Store::Steam, r"C:\G"), game(Store::Epic, r"D:\G")], &fw);
        assert_eq!(view.screen, Screen::Mods);
        assert_eq!(view.chip, "SMF 2.33.45");
        assert!(!view.chip_warn);
        assert_eq!(view.summary, "2 mods · 1 on");
        assert_eq!(
            view.rows[1],
            Row {
                heading: "".into(),
                id: "Id.One".into(),
                name: "One".into(),
                detail: "1.0.0 · A, B".into(),
                position: 1,
                badge: "".into(),
                problem: "".into(),
                can_toggle: true,
            }
        );
        assert_eq!(view.rows[2].position, 0);
        assert_eq!(view.notes.len(), 2);
        assert!(view.notes[0].contains("Epic Games"));
    }

    #[test]
    fn mods_are_grouped_by_kind_then_sorted_by_name() {
        let fw = installed(vec![
            entry("zeta", ModKind::Other, Some(1)),
            entry("Bravo", ModKind::Sound, None),
            entry("alpha", ModKind::Sound, Some(2)),
            entry("Gamma", ModKind::Gameplay, None),
        ]);
        let view = build(&[game(Store::Steam, r"C:\G")], &fw);
        let labels: Vec<String> = view
            .rows
            .iter()
            .map(|r| if r.heading.is_empty() { r.name.clone() } else { format!("# {}", r.heading) })
            .collect();
        assert_eq!(labels, ["# Gameplay", "Gamma", "# Sound", "alpha", "Bravo", "# Other", "zeta"]);
    }

    #[test]
    fn a_broken_mod_can_only_be_turned_off() {
        let mut off = entry("Off", ModKind::Other, None);
        off.problem = Some("bad manifest".into());
        let mut on = entry("On", ModKind::Other, Some(1));
        on.problem = Some("bad manifest".into());
        let view = build(&[game(Store::Steam, r"C:\G")], &installed(vec![off, on]));
        let toggles: Vec<(String, bool)> =
            view.rows.iter().filter(|r| r.heading.is_empty()).map(|r| (r.name.clone(), r.can_toggle)).collect();
        assert_eq!(toggles, [("Off".to_string(), false), ("On".to_string(), true)]);
    }

    #[test]
    fn unknown_release_is_flagged() {
        let fw = Fake(Inspection {
            status: Status::Installed {
                folder: PathBuf::from("x"),
                version: Version::Unknown { sha256: "abcdef0123456789abcdef".into() },
            },
            mods: vec![],
            notes: vec![],
        });
        let view = build(&[game(Store::Folder, "x")], &fw);
        assert!(view.chip_warn);
        assert_eq!(view.chip, "SMF unknown version (abcdef012345)");
        assert_eq!(view.summary, "No mods in SMF's Mods folder yet");
        assert!(view.rows.is_empty());
    }
}
