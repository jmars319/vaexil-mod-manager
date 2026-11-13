//! The public contract (ADR 0003, section 2): the JSON every front end other
//! than the window speaks. `vmm --json` prints these, and the MCP server will
//! return them. `vmm schema` writes them as JSON Schema files into `schema/`,
//! which CI compares with the committed copy.
//!
//! These are view types, separate from the backend's own types, so a backend
//! can change inside without changing what tools see. A change that breaks a
//! reader bumps [`SCHEMA_VERSION`].

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::backend::ModKind;
use crate::error::{ErrorCode, VmmError};

/// The version of these types. Readers should check it and refuse a newer
/// major version than they know.
pub const SCHEMA_VERSION: u32 = 1;

/// A game install VMM found.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct GameView {
    /// A stable id for the game, such as `hitman-woa`.
    pub id: String,
    pub name: String,
    /// Where it was found: Steam, Epic Games, Microsoft Store or Folder.
    pub store: String,
    /// The game folder.
    pub folder: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct GameList {
    pub schema_version: u32,
    /// Every install found. Commands act on the first one unless `--game`
    /// names another.
    pub games: Vec<GameView>,
}

/// Whether the mod framework is there and readable.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum BackendState {
    Installed,
    NotInstalled,
    Unreadable,
}

/// How changes reach the game.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum DeployMethod {
    /// The framework's own tool builds the game's files (SMF's `Deploy.exe`).
    ExternalTool,
    /// VMM places the files itself.
    PlaceFiles,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Capabilities {
    /// The load orders the backend keeps, such as `["smf"]`.
    pub orders: Vec<String>,
    /// Whether mods can have options.
    pub options: bool,
    pub deploy: DeployMethod,
    /// Whether a change reaches the game only after a deploy.
    pub needs_deploy: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum NativeModsStatus {
    /// The game has no mod system of its own.
    None,
    /// VMM shows the game's own mods read-only and works around them.
    Coexists,
    /// As `coexists`, with the limits given in the explanation.
    CoexistsWithLimits,
    /// VMM can't work safely alongside the game's own mods.
    Unsupported,
}

/// How VMM lives alongside the game's own mod system (ADR 0003, section 5).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct NativeMods {
    pub status: NativeModsStatus,
    pub explanation: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct BackendView {
    /// A stable id, such as `smf`.
    pub id: String,
    pub name: String,
    pub state: BackendState,
    /// The framework's version, when it is one this build knows.
    pub version: Option<String>,
    /// For a version this build doesn't know: the hash that identifies it.
    pub unknown_version_sha256: Option<String>,
    pub folder: Option<String>,
    /// Why the framework can't be used, when `state` is `unreadable`.
    pub problem: Option<String>,
    pub capabilities: Capabilities,
    pub native_mods: NativeMods,
}

/// Where a mod's type comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum KindSource {
    /// Worked out from the mod's files.
    Files,
    /// Chosen by the user.
    User,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct OptionView {
    /// What to pass in `setOptions.chosen` to choose this option.
    pub value: String,
    pub label: String,
    /// Set when this is one choice of a group: exactly one of a group is chosen.
    pub group: Option<String>,
    pub tooltip: Option<String>,
    pub chosen: bool,
    /// Mods this option needs turned on that are off. It can't be chosen
    /// until they are on.
    pub needs: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ModView {
    pub id: String,
    pub name: String,
    pub version: Option<String>,
    pub authors: Vec<String>,
    /// What the mod mostly changes. Information only: it never changes what loads.
    pub kind: ModKind,
    pub kind_source: KindSource,
    pub enabled: bool,
    /// 1-based place in the load order; absent when the mod is off.
    pub position: Option<usize>,
    /// A short framework-specific label, such as `RPKG`.
    pub badge: Option<String>,
    /// Why the mod's own description can't be read. Such a mod can't be turned on.
    pub problem: Option<String>,
    pub options: Vec<OptionView>,
    /// Mods the author says this one must load before.
    pub load_before: Vec<String>,
    /// Mods the author says this one must load after.
    pub load_after: Vec<String>,
    /// Enabled mods later in the order that replace a file this one also
    /// replaces, so their copy is used.
    pub replaced_by: Vec<String>,
    pub folder: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ModList {
    pub schema_version: u32,
    pub game: GameView,
    /// The mods that are on, in load order, then the mods that are off, by name.
    pub mods: Vec<ModView>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ModDetail {
    pub schema_version: u32,
    pub game: GameView,
    #[serde(rename = "mod")]
    pub mod_view: ModView,
}

/// When the game last got the current mods.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct DeployState {
    /// Whether the last successful deploy was of exactly the current mods and
    /// settings. Absent when that can't be worked out.
    pub up_to_date: Option<bool>,
    /// Whether the last deploy succeeded. Absent when there has been none.
    pub last_success: Option<bool>,
    /// When the last deploy finished, in milliseconds since 1970 (UTC).
    pub last_at_ms: Option<u64>,
    /// Whether a deploy, or the game, is running now.
    pub busy: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct StatusReport {
    pub schema_version: u32,
    pub app_version: String,
    pub game: Option<GameView>,
    pub backend: Option<BackendView>,
    pub installed: usize,
    pub enabled: usize,
    /// Mods whose description can't be read.
    pub with_problems: usize,
    /// Problems worth knowing that don't stop the framework working.
    pub notes: Vec<String>,
    pub deploy: Option<DeployState>,
    /// Saved setups, by name.
    pub setups: Vec<String>,
    /// Whether a compatibility pass is under way.
    pub pass_active: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum RelationView {
    Before,
    After,
}

/// An author's ordering rule that the current order breaks.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct BrokenRuleView {
    #[serde(rename = "mod")]
    pub mod_id: String,
    /// `mod` should load `relation` `other`.
    pub relation: RelationView,
    pub other: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct OrderReport {
    pub schema_version: u32,
    pub game: GameView,
    /// The order the backend keeps (`smf`).
    pub order_name: String,
    /// Mod ids, first to last. A later mod wins.
    pub order: Vec<String>,
    pub broken_rules: Vec<BrokenRuleView>,
    /// The order that would keep every rule, moving as little as possible.
    /// Absent when the rules contradict each other.
    pub suggested: Option<Vec<String>>,
    /// Mods the rules can't order, when they contradict each other.
    pub contradictory: Vec<String>,
}

/// One change to make. Every change is planned first and applied by its plan.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "op", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum Change {
    /// Turn a mod on (placed where its rules want it) or off.
    SetEnabled {
        #[serde(rename = "mod")]
        mod_id: String,
        enabled: bool,
    },
    /// Choose exactly these options of a mod, by their `value`s.
    SetOptions {
        #[serde(rename = "mod")]
        mod_id: String,
        chosen: Vec<String>,
    },
    /// Set the type shown for a mod, or with no `kind`, go back to the type
    /// worked out from its files.
    SetKind {
        #[serde(rename = "mod")]
        mod_id: String,
        #[serde(default)]
        kind: Option<ModKind>,
    },
    /// A new load order. It must hold exactly the mods that are on at that
    /// point of the plan.
    Reorder { order: Vec<String> },
    /// The order that keeps every mod's load rules, moving as little as
    /// possible (as the window's Auto-sort).
    AutoSort {},
    /// Make a saved setup the game's mods: exactly its mods on, in its order,
    /// with its options. Planned on its own.
    UseSetup { name: String },
    /// Save the mods that are on, in their order with their options, as a
    /// setup (replacing one of the same name). Planned on its own.
    SaveSetup { name: String },
    /// Delete a saved setup. Planned on its own.
    DeleteSetup { name: String },
    /// Start a compatibility pass: from every mod off, or from the mods on
    /// now as the baseline. Planned on its own.
    StartPass {
        #[serde(default)]
        from_current: bool,
    },
    /// Try a batch of mods on top of the pass's baseline. Planned on its own.
    TryBatch {
        #[serde(default)]
        name: String,
        mods: Vec<String>,
    },
    /// Mark the batch being tried: good joins the baseline, bad puts the game
    /// back to the baseline. Planned on its own.
    MarkBatch {
        good: bool,
        #[serde(default)]
        notes: String,
    },
    /// End the pass, keeping the mods as they are. Planned on its own.
    EndPass {},
}

impl Change {
    /// Setup and pass steps are planned alone, one per plan.
    pub fn stands_alone(&self) -> bool {
        !matches!(
            self,
            Change::SetEnabled { .. }
                | Change::SetOptions { .. }
                | Change::SetKind { .. }
                | Change::Reorder { .. }
                | Change::AutoSort {}
        )
    }
}

/// A file a plan depends on, and its contents when it was made.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Precondition {
    pub path: String,
    /// The SHA-256 of the file, or absent when there was no file.
    pub sha256: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct PlanStep {
    /// What the step does, in plain words.
    pub description: String,
    /// False when the game is already that way, so the step writes nothing.
    pub changes_something: bool,
}

/// A checked set of changes, waiting to be applied by its `planId`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Plan {
    pub schema_version: u32,
    /// The handle to apply it with. A plan is used once.
    pub plan_id: String,
    /// The game folder it is for.
    pub game: String,
    pub changes: Vec<Change>,
    /// One step per change, in order.
    pub steps: Vec<PlanStep>,
    /// Applying refuses if any of these files has changed since.
    pub preconditions: Vec<Precondition>,
    pub created_at_ms: u64,
    /// After this (10 minutes on), the plan can't be applied; plan again.
    pub expires_at_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ApplyReport {
    pub schema_version: u32,
    pub plan_id: String,
    pub steps: Vec<PlanStep>,
    /// Whether the game needs a deploy for the changes to take effect.
    pub needs_deploy: bool,
    /// Set when the changes were made but couldn't be recorded in the journal.
    pub journal_problem: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ErrorBody {
    /// A stable code, such as `config.changed_meanwhile`.
    pub code: String,
    /// What went wrong and what was (not) changed, for people.
    pub message: String,
    /// What to do about it.
    pub hint: String,
    /// Whether trying again later, without changing anything, can work.
    pub retryable: bool,
    /// The process exit code `vmm` uses for this error.
    pub exit_code: i32,
}

/// What every interface returns instead of a result when something fails.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ErrorReport {
    pub schema_version: u32,
    pub error: ErrorBody,
}

impl From<&VmmError> for ErrorReport {
    fn from(e: &VmmError) -> ErrorReport {
        ErrorReport {
            schema_version: SCHEMA_VERSION,
            error: ErrorBody {
                code: e.code.as_str().into(),
                message: e.message.clone(),
                hint: e.code.hint().into(),
                retryable: e.code.retryable(),
                exit_code: e.code.exit_code(),
            },
        }
    }
}

/// One entry of the list of error codes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ErrorCodeInfo {
    pub code: String,
    pub meaning: String,
    pub hint: String,
    pub retryable: bool,
    pub exit_code: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ErrorCodeList {
    pub schema_version: u32,
    pub codes: Vec<ErrorCodeInfo>,
}

impl ErrorCodeList {
    pub fn current() -> ErrorCodeList {
        ErrorCodeList {
            schema_version: SCHEMA_VERSION,
            codes: ErrorCode::ALL
                .iter()
                .map(|c| ErrorCodeInfo {
                    code: c.as_str().into(),
                    meaning: c.meaning().into(),
                    hint: c.hint().into(),
                    retryable: c.retryable(),
                    exit_code: c.exit_code(),
                })
                .collect(),
        }
    }
}

/// How serious a logged event is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum LogLevel {
    Info,
    Warn,
    Error,
}

/// One line of the app's event log (`logs\vmm-<date>.jsonl`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct LogEvent {
    /// Milliseconds since 1970 (UTC).
    pub ts_ms: u64,
    pub level: LogLevel,
    /// What happened, such as `mod.enabled`, `plan.applied` or `deploy.finished`.
    pub event: String,
    /// Who made it happen: `gui`, `cli` or `mcp:<AI app name>`.
    pub caller: String,
    pub game: Option<String>,
    pub backend: Option<String>,
    #[serde(rename = "mod")]
    pub mod_id: Option<String>,
    /// The plan, when the event is part of one.
    pub plan: Option<String>,
    pub message: String,
    /// Set when the event is a failure.
    pub error: Option<ErrorBody>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct LogList {
    pub schema_version: u32,
    /// Newest last.
    pub events: Vec<LogEvent>,
}

/// How a deploy ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum DeployResult {
    Succeeded,
    Failed,
    /// The deploy tool could not be started, or ended without saying.
    DidNotRun,
}

/// A stage the deploy tool reported, in order.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct DeployStage {
    pub name: String,
    /// The mod the stage was for, when it was for one.
    #[serde(rename = "mod")]
    pub mod_id: Option<String>,
}

/// Warnings of one kind about one mod, counted once each.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct DeployWarning {
    /// A stable code such as `smf.raw_entity_file`, or `smf.warning` for one
    /// this version doesn't recognise.
    pub code: String,
    /// The mod's name as the deploy tool gave it, when it named one.
    #[serde(rename = "mod")]
    pub mod_name: Option<String>,
    /// How many distinct warnings of this kind.
    pub count: usize,
    /// What it means, in plain words.
    pub detail: String,
    /// Up to five of the warnings as the tool wrote them.
    pub examples: Vec<String>,
}

/// The last deploy, read from the deploy tool's own output. A line the
/// parser doesn't recognise stays only in the raw log and never fails it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct DeployReport {
    pub schema_version: u32,
    pub result: DeployResult,
    pub exit_code: Option<i32>,
    pub started_at_ms: Option<u64>,
    pub finished_at_ms: Option<u64>,
    pub duration_ms: Option<u64>,
    pub stages: Vec<DeployStage>,
    pub warnings: Vec<DeployWarning>,
    /// Lines the tool marked as errors, and Vaexil Mod Manager's own reason
    /// when the tool could not run.
    pub errors: Vec<String>,
    /// The full log.
    pub log: Option<String>,
    pub log_lines: usize,
}

/// A saved setup.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SetupView {
    pub name: String,
    /// Mod ids, first to last.
    pub order: Vec<String>,
    /// Chosen options by mod id.
    pub options: std::collections::BTreeMap<String, Vec<String>>,
    /// Whether the mods on now are exactly this setup's, in its order.
    pub in_use: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum BatchState {
    Testing,
    Good,
    Bad,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct BatchView {
    pub name: String,
    pub mods: Vec<String>,
    pub state: BatchState,
    pub notes: String,
}

/// A compatibility pass: the known-good baseline and each batch tried.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct PassView {
    pub baseline: Vec<String>,
    pub batches: Vec<BatchView>,
    /// Mods that can go into the next batch: installed, readable, not in the
    /// baseline.
    pub candidates: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SetupList {
    pub schema_version: u32,
    pub setups: Vec<SetupView>,
    /// The compatibility pass under way, if any.
    pub pass: Option<PassView>,
}

/// A deploy started from the command line or an AI tool, while it runs and
/// once it has finished.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct DeployStatus {
    pub schema_version: u32,
    pub running: bool,
    /// The deploy tool's latest output line.
    pub last_line: String,
    /// Set once it has finished.
    pub report: Option<DeployReport>,
}

/// `vmm deploy-report`: the last deploy, or none yet.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct LastDeploy {
    pub schema_version: u32,
    pub report: Option<DeployReport>,
}

/// A problem worth knowing, from anywhere in the app.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Problem {
    /// A stable code, such as `mod.unreadable` or `order.rule_broken`.
    pub code: String,
    #[serde(rename = "mod")]
    pub mod_id: Option<String>,
    pub detail: String,
}

/// Everything needed to understand a problem, in one object. Paths under
/// the user's profile start with `~`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Diagnosis {
    pub schema_version: u32,
    pub app_version: String,
    pub generated_at_ms: u64,
    /// Where the game was looked for and what was found.
    pub detection: Vec<String>,
    pub status: StatusReport,
    pub problems: Vec<Problem>,
    pub last_deploy: Option<DeployReport>,
    /// The newest 200 events, newest last.
    pub recent_events: Vec<LogEvent>,
    /// The app's settings that change behaviour.
    pub settings: serde_json::Value,
    /// Crash records the app has written, newest first, at most five.
    pub crashes: Vec<String>,
}

/// Every type above that a command prints, by the file name `vmm schema`
/// writes it to.
pub fn all() -> Vec<(&'static str, schemars::Schema)> {
    vec![
        ("game-list", schemars::schema_for!(GameList)),
        ("status-report", schemars::schema_for!(StatusReport)),
        ("mod-list", schemars::schema_for!(ModList)),
        ("mod-detail", schemars::schema_for!(ModDetail)),
        ("order-report", schemars::schema_for!(OrderReport)),
        ("change", schemars::schema_for!(Change)),
        ("plan", schemars::schema_for!(Plan)),
        ("apply-report", schemars::schema_for!(ApplyReport)),
        ("error-report", schemars::schema_for!(ErrorReport)),
        ("error-code-list", schemars::schema_for!(ErrorCodeList)),
        ("log-list", schemars::schema_for!(LogList)),
        ("last-deploy", schemars::schema_for!(LastDeploy)),
        ("setup-list", schemars::schema_for!(SetupList)),
        ("deploy-status", schemars::schema_for!(DeployStatus)),
        ("diagnosis", schemars::schema_for!(Diagnosis)),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn changes_use_the_documented_json() {
        let c: Change = serde_json::from_str(r#"{ "op": "setEnabled", "mod": "A", "enabled": true }"#).unwrap();
        assert_eq!(c, Change::SetEnabled { mod_id: "A".into(), enabled: true });
        let c: Change = serde_json::from_str(r#"{ "op": "setKind", "mod": "A" }"#).unwrap();
        assert_eq!(c, Change::SetKind { mod_id: "A".into(), kind: None });
        let c: Change = serde_json::from_str(r#"{ "op": "setKind", "mod": "A", "kind": "models-textures" }"#).unwrap();
        assert_eq!(c, Change::SetKind { mod_id: "A".into(), kind: Some(ModKind::ModelsTextures) });
        assert!(serde_json::from_str::<Change>(r#"{ "op": "deleteEverything" }"#).is_err());
        let json = serde_json::to_value(Change::Reorder { order: vec!["A".into()] }).unwrap();
        assert_eq!(json, serde_json::json!({ "op": "reorder", "order": ["A"] }));
    }

    #[test]
    fn kinds_serialize_as_their_saved_keys() {
        for k in ModKind::ALL {
            assert_eq!(serde_json::to_value(k).unwrap(), serde_json::Value::String(k.key().into()));
        }
    }

    #[test]
    fn every_error_code_has_a_hint_and_an_exit_code() {
        let list = ErrorCodeList::current();
        assert_eq!(list.codes.len(), ErrorCode::ALL.len());
        for c in &list.codes {
            assert!(!c.hint.is_empty() && !c.meaning.is_empty(), "{}", c.code);
            assert!([2, 3, 4, 5, 10].contains(&c.exit_code), "{}", c.code);
        }
        let mut codes: Vec<&str> = list.codes.iter().map(|c| c.code.as_str()).collect();
        codes.sort();
        codes.dedup();
        assert_eq!(codes.len(), ErrorCode::ALL.len(), "codes are unique");
    }
}
