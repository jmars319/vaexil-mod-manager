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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModEntry {
    pub id: String,
    pub name: String,
    pub version: Option<String>,
    pub authors: Vec<String>,
    pub folder: PathBuf,
    /// A short framework-specific label, such as "RPKG".
    pub badge: Option<String>,
    /// 1-based place in the load order; `None` when the mod is off.
    pub load_position: Option<usize>,
    /// Why the mod's own description could not be read, if it could not.
    pub problem: Option<String>,
}
