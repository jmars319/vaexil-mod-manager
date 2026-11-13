//! What every game backend reports and can do. Front ends see only these
//! types (through the engine), so a second backend can be added without
//! changing them. Version 1 has one backend, `vmm-backend-smf`.

use std::collections::BTreeMap;
use std::path::PathBuf;

use crate::error::VmmError;
use crate::game::GameInstall;
use crate::schema::{Capabilities, NativeMods};

pub trait Backend {
    /// A stable id for tools and saved files, such as `smf`.
    fn id(&self) -> &'static str;

    /// The name shown in the window, such as "Simple Mod Framework".
    fn name(&self) -> &'static str;

    /// What the backend can do: its load orders, options, and how it deploys.
    fn capabilities(&self) -> Capabilities;

    /// How the backend lives alongside the game's own mod system.
    fn native_mods(&self) -> NativeMods;

    /// The files the backend's settings live in for this install, and the
    /// description files of `mods` (whose options and defaults a plan
    /// depends on). A plan records their hashes and applying it refuses if
    /// any has changed.
    fn state_files(&self, game: &GameInstall, mods: &[String]) -> Vec<PathBuf>;

    /// Reads the framework's state for one game install. Never writes.
    fn inspect(&self, game: &GameInstall) -> Inspection;

    /// Turns one mod on or off in the framework's own settings. Returns
    /// `false` when the mod was already in that state and nothing was written.
    fn set_enabled(&self, game: &GameInstall, mod_id: &str, enabled: bool) -> Result<bool, VmmError>;

    /// Saves which of a mod's options are chosen, given as their
    /// [`ModOption::value`]s. Returns `false` when nothing changed.
    fn set_options(&self, game: &GameInstall, mod_id: &str, chosen: &[String]) -> Result<bool, VmmError>;

    /// The options `set_options` would store for `chosen` if the mods on
    /// were `load_order`: checked, in the mod's order, with every select
    /// group filled. Writes nothing; plans use it to describe exactly what
    /// will be written.
    fn resolve_options(
        &self,
        game: &GameInstall,
        mod_id: &str,
        chosen: &[String],
        load_order: &[String],
    ) -> Result<Vec<String>, VmmError>;

    /// Saves a new load order. It must hold exactly the mods that are on now,
    /// first to last; anything else is refused. Returns `false` when the order
    /// was already this.
    fn set_load_order(&self, game: &GameInstall, order: &[String]) -> Result<bool, VmmError>;

    /// The mods that are on, in order, with their chosen options.
    fn current_setup(&self, game: &GameInstall) -> Result<Setup, VmmError>;

    /// Makes `setup` the framework's state: exactly its mods on, in its order,
    /// with its options. Mods it names that are no longer installed are left
    /// out and returned.
    fn apply_setup(&self, game: &GameInstall, setup: &Setup) -> Result<Vec<String>, VmmError>;

    /// Changes whenever something a deploy reads has changed.
    fn deploy_fingerprint(&self, game: &GameInstall) -> Result<String, VmmError>;

    /// Runs the framework's own deploy, passing each output line to `on_line`.
    fn deploy(&self, game: &GameInstall, on_line: &mut dyn FnMut(&str)) -> Result<DeployOutcome, VmmError>;
}

/// A saved state: which mods are on, in what order, with what options.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Setup {
    /// Mod ids, first to last.
    pub order: Vec<String>,
    /// Chosen options by mod id, as the framework stores them.
    pub options: BTreeMap<String, Vec<String>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeployOutcome {
    pub success: bool,
    /// `None` when the process was ended without an exit code.
    pub exit_code: Option<i32>,
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
#[derive(
    Debug,
    Clone,
    Copy,
    Default,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
    serde::Serialize,
    serde::Deserialize,
    schemars::JsonSchema,
)]
#[serde(rename_all = "kebab-case")]
pub enum ModKind {
    Gameplay,
    ModelsTextures,
    Sound,
    Dialogue,
    Ui,
    Text,
    #[default]
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

#[derive(Debug, Clone, Default, PartialEq, Eq)]
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
    /// Where the mod's author says it must load relative to other mods,
    /// counting only rules that apply to the versions installed.
    pub rules: Vec<OrderRule>,
    /// Game files the mod replaces wholesale, as framework-specific keys. Two
    /// enabled mods with the same key: the later one's file is used.
    pub replaces: Vec<String>,
}

/// "Load before" or "load after" another mod.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Relation {
    Before,
    After,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OrderRule {
    pub relation: Relation,
    /// The other mod's id.
    pub other: String,
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
