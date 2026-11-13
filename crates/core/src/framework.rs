//! What every mod framework module reports. The window only sees these types,
//! so a second framework can be added without changing it. Version 1 has one
//! module, [`crate::smf::Smf`].

use std::path::PathBuf;

use crate::game::GameInstall;

pub trait Framework {
    /// The name shown in the window, such as "Simple Mod Framework".
    fn name(&self) -> &'static str;

    /// Reads the framework's state for one game install. Never writes.
    fn inspect(&self, game: &GameInstall) -> Inspection;

    /// Turns one mod on or off in the framework's own settings. Returns
    /// `false` when the mod was already in that state and nothing was written.
    fn set_enabled(&self, game: &GameInstall, mod_id: &str, enabled: bool) -> Result<bool, String>;

    /// Saves which of a mod's options are chosen, given as their
    /// [`ModOption::value`]s. Returns `false` when nothing changed.
    fn set_options(&self, game: &GameInstall, mod_id: &str, chosen: &[String]) -> Result<bool, String>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Inspection {
    pub status: Status,
    /// In load order first, then the mods that are off, by name.
    pub mods: Vec<ModEntry>,
    /// Problems worth showing that do not stop the framework from working.
    pub notes: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Status {
    NotInstalled,
    /// The framework folder exists but its settings could not be read.
    Unreadable {
        folder: PathBuf,
        reason: String,
    },
    Installed {
        folder: PathBuf,
        version: Version,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Version {
    Known(String),
    /// Not one of the releases this build knows. The hash identifies it.
    Unknown {
        sha256: String,
    },
}

impl Version {
    pub fn label(&self) -> String {
        match self {
            Version::Known(v) => v.clone(),
            Version::Unknown { sha256 } => format!("unknown version ({})", &sha256[..sha256.len().min(12)]),
        }
    }
}

/// What a mod mostly changes, for grouping the Mods view. Information only:
/// it never affects what the framework loads.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ModKind {
    Gameplay,
    ModelsTextures,
    Sound,
    Dialogue,
    Ui,
    Text,
    Other,
}

impl ModKind {
    /// Every kind, in the order the Mods view shows its groups.
    pub const ALL: [ModKind; 7] = [
        ModKind::Gameplay,
        ModKind::ModelsTextures,
        ModKind::Sound,
        ModKind::Dialogue,
        ModKind::Ui,
        ModKind::Text,
        ModKind::Other,
    ];

    /// A stable name for saving the kind, such as "models-textures".
    pub fn key(self) -> &'static str {
        match self {
            ModKind::Gameplay => "gameplay",
            ModKind::ModelsTextures => "models-textures",
            ModKind::Sound => "sound",
            ModKind::Dialogue => "dialogue",
            ModKind::Ui => "ui",
            ModKind::Text => "text",
            ModKind::Other => "other",
        }
    }

    pub fn from_key(key: &str) -> Option<ModKind> {
        ModKind::ALL.into_iter().find(|k| k.key() == key)
    }

    pub fn label(self) -> &'static str {
        match self {
            ModKind::Gameplay => "Gameplay",
            ModKind::ModelsTextures => "Models & Textures",
            ModKind::Sound => "Sound",
            ModKind::Dialogue => "Dialogue",
            ModKind::Ui => "UI",
            ModKind::Text => "Text",
            ModKind::Other => "Other",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModEntry {
    pub id: String,
    pub name: String,
    pub version: Option<String>,
    pub authors: Vec<String>,
    pub folder: PathBuf,
    /// A short framework-specific label, such as "RPKG".
    pub badge: Option<String>,
    pub kind: ModKind,
    /// 1-based place in the load order; `None` when the mod is off.
    pub load_position: Option<usize>,
    /// Why the mod's own description could not be read, if it could not.
    /// Such a mod cannot be turned on.
    pub problem: Option<String>,
    /// The options the user can choose, in the mod's own order.
    pub options: Vec<ModOption>,
}

/// One choosable option of a mod.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModOption {
    /// What the framework stores when the option is chosen.
    pub value: String,
    pub label: String,
    /// Set when the option is one choice among several: exactly one option
    /// of a group is chosen.
    pub group: Option<String>,
    pub tooltip: Option<String>,
    pub chosen: bool,
    /// Mods this option needs turned on, which are not. It can't be chosen
    /// until they are.
    pub needs: Vec<String>,
}
