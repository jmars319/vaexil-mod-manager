//! The engine's operations in the public JSON contract (`vmm_core::schema`):
//! the read views every front end other than the window uses, and planning
//! and applying changes (ADR 0003, sections 2 and 3).
//!
//! Every change is planned first. A plan checks each change against the
//! state it will meet, records the hashes of the files it depends on, and is
//! saved under `plans\` in the data folder. Applying it rechecks those hashes
//! and refuses on any difference, so nothing is applied to a state the caller
//! didn't see. A plan expires after 10 minutes and is used once.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};
use vmm_core::backend::{Inspection, ModEntry, ModKind, Relation, Status, Version};
use vmm_core::error::{ErrorCode, VmmError};
use vmm_core::game::GameInstall;
use vmm_core::schema::{
    ApplyReport, BackendState, BackendView, BatchState, BatchView, BrokenRuleView, Change, DeployState, DeployStatus,
    GameList, GameView, KindSource, LogLevel, ModDetail, ModList, ModSummary, ModView, OptionView, OrderReport,
    PassView, Plan, PlanStep, Precondition, RelationView, SCHEMA_VERSION, SetupList, SetupView, StatusReport,
};

use crate::journal::{self, Caller};
use crate::{Engine, deploy, enabled_order, host, settings, setups};

/// How long a plan can be applied for.
pub const PLAN_LIFETIME_MS: u64 = 10 * 60 * 1000;

const PLANS: &str = "plans";

pub fn game_view(game: &GameInstall) -> GameView {
    GameView {
        id: "hitman-woa".into(),
        name: "HITMAN World of Assassination".into(),
        store: game.store.label().into(),
        folder: game.root.display().to_string(),
    }
}

fn mod_view(m: &ModEntry, kinds: &BTreeMap<String, ModKind>, replaced_by: &BTreeMap<String, Vec<String>>) -> ModView {
    let user_kind = kinds.get(&m.id).copied();
    let rules = |rel: Relation| m.rules.iter().filter(|r| r.relation == rel).map(|r| r.other.clone()).collect();
    ModView {
        id: m.id.clone(),
        name: m.name.clone(),
        version: m.version.clone(),
        authors: m.authors.clone(),
        kind: user_kind.unwrap_or(m.kind),
        kind_source: if user_kind.is_some() { KindSource::User } else { KindSource::Files },
        enabled: m.load_position.is_some(),
        position: m.load_position,
        badge: m.badge.clone(),
        problem: m.problem.clone(),
        options: m
            .options
            .iter()
            .map(|o| OptionView {
                value: o.value.clone(),
                label: o.label.clone(),
                group: o.group.clone(),
                tooltip: o.tooltip.clone(),
                chosen: o.chosen,
                needs: o.needs.clone(),
            })
            .collect(),
        load_before: rules(Relation::Before),
        load_after: rules(Relation::After),
        replaced_by: replaced_by.get(&m.id).cloned().unwrap_or_default(),
        folder: m.folder.display().to_string(),
    }
}

fn relation_view(r: Relation) -> RelationView {
    match r {
        Relation::Before => RelationView::Before,
        Relation::After => RelationView::After,
    }
}

fn sha256_of(path: &Path) -> Result<Option<String>, VmmError> {
    match std::fs::read(path) {
        Ok(bytes) => Ok(Some(hex(&bytes))),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(VmmError::new(ErrorCode::Io, format!("Couldn't read {}: {e}", path.display()))),
    }
}

fn hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect()
}

/// A plan id is 16 lowercase hex digits; nothing else names a plan file.
fn valid_plan_id(id: &str) -> bool {
    id.len() == 16 && id.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

fn mod_label(m: &ModEntry) -> String {
    if m.name == m.id {
        format!("\u{201c}{}\u{201d}", m.name)
    } else {
        format!("\u{201c}{}\u{201d} ({})", m.name, m.id)
    }
}

/// The state a plan's changes meet, one after another.
struct Simulated {
    order: Vec<String>,
    /// Each mod's stored options, as stored: absent when none are stored,
    /// so storing the shown defaults counts as a change, as it is.
    chosen: BTreeMap<String, Vec<String>>,
    kinds: BTreeMap<String, ModKind>,
}

impl Engine {
    /// The game a command acts on: the folder named, or the first install found.
    pub fn pick_game(&self, folder: Option<&Path>) -> Result<GameInstall, VmmError> {
        if let Some(folder) = folder {
            if !vmm_core::game::is_game_root(folder) {
                return Err(VmmError::new(
                    ErrorCode::GameNotFound,
                    format!("{} isn't a HITMAN folder: it has no Retail\\HITMAN3.exe.", folder.display()),
                ));
            }
            // Absolute, so a plan names the same folder whatever the
            // current directory is when it is applied.
            let root = std::path::absolute(folder).map_err(|e| {
                VmmError::new(ErrorCode::GameNotFound, format!("{} can't be used: {e}", folder.display()))
            })?;
            return Ok(GameInstall { store: vmm_core::game::Store::Folder, root });
        }
        self.installs().into_iter().next().ok_or_else(|| {
            VmmError::new(ErrorCode::GameNotFound, "No install of HITMAN World of Assassination was found.")
        })
    }

    pub fn games(&self) -> GameList {
        GameList { schema_version: SCHEMA_VERSION, games: self.installs().iter().map(game_view).collect() }
    }

    pub fn backend_view(&self, inspection: &Inspection) -> BackendView {
        let b = self.backend();
        let (state, version, unknown, folder, problem) = match &inspection.status {
            Status::NotInstalled => (BackendState::NotInstalled, None, None, None, None),
            Status::Unreadable { folder, reason } => {
                (BackendState::Unreadable, None, None, Some(folder.display().to_string()), Some(reason.clone()))
            }
            Status::Installed { folder, version } => {
                let (known, unknown) = match version {
                    Version::Known(v) => (Some(v.clone()), None),
                    Version::Unknown { sha256 } => (None, Some(sha256.clone())),
                };
                (BackendState::Installed, known, unknown, Some(folder.display().to_string()), None)
            }
        };
        BackendView {
            id: b.id().into(),
            name: b.name().into(),
            state,
            version,
            unknown_version_sha256: unknown,
            folder,
            problem,
            capabilities: b.capabilities(),
            native_mods: b.native_mods(),
        }
    }

    /// The mods, after checking the framework can be read.
    fn readable(&self, game: &GameInstall) -> Result<Inspection, VmmError> {
        let inspection = self.inspect(game);
        match &inspection.status {
            Status::Installed { .. } => Ok(inspection),
            Status::NotInstalled => Err(VmmError::new(
                ErrorCode::FrameworkMissing,
                format!("{} isn't installed in {}.", self.backend().name(), game.root.display()),
            )),
            Status::Unreadable { reason, .. } => Err(VmmError::new(ErrorCode::ConfigUnreadable, reason.clone())),
        }
    }

    /// Everything at a glance. Never fails: what can't be read is left out.
    pub fn status(&self, game: Option<&GameInstall>) -> StatusReport {
        let setups = self.setups().ok();
        let mut report = StatusReport {
            schema_version: SCHEMA_VERSION,
            app_version: env!("CARGO_PKG_VERSION").into(),
            game: game.map(game_view),
            backend: None,
            installed: 0,
            enabled: 0,
            with_problems: 0,
            notes: vec![],
            deploy: None,
            setups: setups.as_ref().map(|s| s.saved.iter().map(|x| x.name.clone()).collect()).unwrap_or_default(),
            pass_active: setups.as_ref().is_some_and(|s| s.pass.is_some()),
        };
        let Some(game) = game else { return report };
        let inspection = self.inspect(game);
        report.backend = Some(self.backend_view(&inspection));
        report.installed = inspection.mods.len();
        report.enabled = inspection.mods.iter().filter(|m| m.load_position.is_some()).count();
        report.with_problems = inspection.mods.iter().filter(|m| m.problem.is_some()).count();
        report.notes = inspection.notes.clone();
        if matches!(inspection.status, Status::Installed { .. }) {
            let record = self.record();
            let current = self.fingerprint(game).ok();
            report.deploy = Some(DeployState {
                up_to_date: current.map(|c| record.fingerprint.as_deref() == Some(c.as_str())),
                last_success: record.success,
                last_at_ms: record.at_ms,
                busy: self.blocker().map(|e| e.message),
            });
        }
        report
    }

    /// Every mod in full, in the list's order.
    fn mod_views(&self, game: &GameInstall) -> Result<Vec<ModView>, VmmError> {
        let inspection = self.readable(game)?;
        let kinds = self.kind_overrides();
        let replaced = vmm_core::order::overridden_by(&enabled_order(&inspection.mods), &inspection.mods);
        Ok(inspection.mods.iter().map(|m| mod_view(m, &kinds, &replaced)).collect())
    }

    pub fn mod_list(&self, game: &GameInstall) -> Result<ModList, VmmError> {
        let mods = self.mod_views(game)?;
        Ok(ModList {
            schema_version: SCHEMA_VERSION,
            game: game_view(game),
            mods: mods.iter().map(ModSummary::from).collect(),
        })
    }

    pub fn mod_detail(&self, game: &GameInstall, mod_id: &str) -> Result<ModDetail, VmmError> {
        let m = self.mod_views(game)?.into_iter().find(|m| m.id == mod_id).ok_or_else(|| no_such_mod(mod_id))?;
        Ok(ModDetail { schema_version: SCHEMA_VERSION, game: game_view(game), mod_view: m, options_left_out: 0 })
    }

    pub fn order_report(&self, game: &GameInstall) -> Result<OrderReport, VmmError> {
        let inspection = self.readable(game)?;
        let order = enabled_order(&inspection.mods);
        let broken = vmm_core::order::broken_rules(&order, &inspection.mods);
        let (suggested, contradictory) = match vmm_core::order::auto_sort(&order, &inspection.mods) {
            Ok(sorted) => (Some(sorted), vec![]),
            Err(stuck) => (None, stuck),
        };
        Ok(OrderReport {
            schema_version: SCHEMA_VERSION,
            game: game_view(game),
            order_name: self.backend().capabilities().orders.first().cloned().unwrap_or_default(),
            order,
            broken_rules: broken
                .into_iter()
                .map(|b| BrokenRuleView { mod_id: b.mod_id, relation: relation_view(b.relation), other: b.other })
                .collect(),
            suggested,
            contradictory,
        })
    }

    // Plans.

    fn plans_dir(&self) -> Result<PathBuf, VmmError> {
        self.data_dir().map(|d| d.join(PLANS)).ok_or_else(|| {
            VmmError::new(
                ErrorCode::BackupUnavailable,
                "Windows didn't say where your local app data folder is, so there is nowhere to keep plans or \
                 backups. Nothing was changed.",
            )
        })
    }

    /// Checks each change against the state the changes before it leave, and
    /// describes it. Writes nothing.
    fn check(&self, game: &GameInstall, changes: &[Change]) -> Result<Vec<PlanStep>, VmmError> {
        if changes.is_empty() {
            return Err(VmmError::new(ErrorCode::Usage, "A plan needs at least one change."));
        }
        if changes.len() > 1 && changes.iter().any(Change::stands_alone) {
            return Err(VmmError::new(
                ErrorCode::Usage,
                "A setup or compatibility pass step is planned on its own, one per plan. Nothing was changed.",
            ));
        }
        let inspection = self.readable(game)?;
        let mods = &inspection.mods;
        let find = |id: &str| mods.iter().find(|m| m.id == id).ok_or_else(|| no_such_mod(id));
        let mut sim = Simulated {
            order: enabled_order(mods),
            chosen: self.backend().stored_options(game)?,
            // A type change needs readable settings: refuse it now rather
            // than after earlier steps have been applied.
            kinds: if changes.iter().any(|c| matches!(c, Change::SetKind { .. })) {
                let dir = self.data_dir().ok_or_else(|| {
                    VmmError::new(
                        ErrorCode::BackupUnavailable,
                        "There is no folder to keep types in. Nothing was changed.",
                    )
                })?;
                settings::Settings::load(dir)?.kind_overrides
            } else {
                self.kind_overrides()
            },
        };
        // Options an earlier step chose. Options bring load rules of their
        // own, and which defaults apply depends on the mods that are on, so
        // once the plan has changed either, placing a mod reads the mods
        // again as they will be at that point.
        let mut chosen_by_plan: BTreeMap<String, Vec<String>> = BTreeMap::new();
        let stored_order = sim.order.clone();
        let mut steps = Vec::new();
        for change in changes {
            let step = match change {
                Change::SetEnabled { mod_id, enabled } => {
                    let m = find(mod_id)?;
                    let on = sim.order.contains(mod_id);
                    // The backend checks a mod it is asked to turn on even when
                    // it is on already, so a plan does too.
                    if *enabled && let Some(problem) = &m.problem {
                        return Err(VmmError::new(
                            ErrorCode::ModUnreadable,
                            format!("{} can't be turned on. {problem}", mod_label(m)),
                        ));
                    }
                    // Where the backend will put it: the same rule-aware
                    // insertion, so the step can say the exact place and a
                    // rule changed after planning changes the step.
                    let mut place = None;
                    if *enabled && !on {
                        let at = if chosen_by_plan.is_empty() && sim.order == stored_order {
                            vmm_core::order::insert_position(&sim.order, mod_id, mods)
                        } else {
                            let now = self.backend().mods_with(game, &sim.order, &chosen_by_plan)?;
                            vmm_core::order::insert_position(&sim.order, mod_id, &now)
                        };
                        // A mod with no stored options gets its defaults,
                        // worked out against the mods on before it, and they
                        // stay stored for later steps.
                        if !chosen_by_plan.contains_key(mod_id)
                            && let Some(d) = self.backend().enable_defaults(game, mod_id, &sim.order)?
                        {
                            sim.chosen.insert(mod_id.clone(), d.clone());
                            chosen_by_plan.insert(mod_id.clone(), d);
                        }
                        sim.order.insert(at, mod_id.clone());
                        place = Some((at + 1, sim.order.len()));
                    } else if !*enabled {
                        sim.order.retain(|id| id != mod_id);
                    }
                    let at = place.map(|(p, n)| format!(", at place {p} of {n} in the load order")).unwrap_or_default();
                    PlanStep {
                        description: format!("Turn {} {}{at}", mod_label(m), if *enabled { "on" } else { "off" }),
                        changes_something: *enabled != on,
                    }
                }
                Change::SetOptions { mod_id, chosen } => {
                    let m = find(mod_id)?;
                    // What the backend would store with the mods on at this
                    // point of the plan: requirements checked against that,
                    // select groups filled as applying will fill them.
                    let resolved = self.backend().resolve_options(game, mod_id, chosen, &sim.order).map_err(|e| {
                        VmmError::new(e.code, format!("{}: {} Nothing was changed.", mod_label(m), e.message))
                    })?;
                    chosen_by_plan.insert(mod_id.clone(), resolved.clone());
                    // As the write compares: the stored list, exactly.
                    let changed = sim.chosen.get(mod_id) != Some(&resolved);
                    sim.chosen.insert(mod_id.clone(), resolved.clone());
                    let list = if resolved.is_empty() { "none".to_string() } else { resolved.join(", ") };
                    PlanStep {
                        description: format!("Choose these options of {}: {list}", mod_label(m)),
                        changes_something: changed,
                    }
                }
                Change::SetKind { mod_id, kind } => {
                    let m = find(mod_id)?;
                    let changed = sim.kinds.get(mod_id) != kind.as_ref();
                    match kind {
                        Some(k) => sim.kinds.insert(mod_id.clone(), *k),
                        None => sim.kinds.remove(mod_id),
                    };
                    let what = match kind {
                        Some(k) => format!("Show {} as {}", mod_label(m), k.label()),
                        None => format!("Show {} as the type its files suggest", mod_label(m)),
                    };
                    PlanStep { description: what, changes_something: changed }
                }
                Change::Reorder { order } => {
                    let want: BTreeSet<&String> = sim.order.iter().collect();
                    let got: BTreeSet<&String> = order.iter().collect();
                    if got.len() != order.len() || want != got {
                        return Err(VmmError::new(
                            ErrorCode::OrderMismatch,
                            format!(
                                "A new load order must list each mod that is on at that point exactly once: {}. \
                                 Nothing was changed.",
                                sim.order.join(", ")
                            ),
                        ));
                    }
                    let changed = *order != sim.order;
                    sim.order = order.clone();
                    PlanStep {
                        description: format!("Load the mods that are on in this order: {}", order.join(", ")),
                        changes_something: changed,
                    }
                }
                Change::AutoSort {} => {
                    let now = self.backend().mods_with(game, &sim.order, &chosen_by_plan)?;
                    let sorted = vmm_core::order::auto_sort(&sim.order, &now).map_err(|stuck| {
                        VmmError::new(
                            ErrorCode::OrderMismatch,
                            format!(
                                "These mods' load rules contradict each other, so no order keeps them all: {}. \
                                 Nothing was changed.",
                                stuck.join(", ")
                            ),
                        )
                    })?;
                    let changed = sorted != sim.order;
                    sim.order = sorted.clone();
                    PlanStep {
                        description: format!("Sort the load order by the mods' rules: {}", sorted.join(", ")),
                        changes_something: changed,
                    }
                }
                alone => self.check_alone(game, mods, &sim.order, alone)?,
            };
            steps.push(step);
        }
        Ok(steps)
    }

    /// The SHA-256 of the backend's settings file (SMF's `config.json`), to
    /// tell whether a step that writes it as a whole changed anything.
    fn config_hash(&self, game: &GameInstall) -> Option<String> {
        self.backend().state_files(game, &[]).first().and_then(|p| sha256_of(p).ok().flatten())
    }

    /// The setups and pass as saved now, read again from disk.
    fn fresh_setups(&self) -> Result<setups::Setups, VmmError> {
        if self.data_dir().is_none() {
            return Err(VmmError::new(ErrorCode::BackupUnavailable, "There is no folder to keep setups in."));
        }
        self.setups()
    }

    /// The saved setup named `name` (ignoring case), by its place in the list.
    fn setup_index(&self, name: &str) -> Result<usize, VmmError> {
        self.fresh_setups()?.saved.iter().position(|s| s.name.eq_ignore_ascii_case(name.trim())).ok_or_else(|| {
            VmmError::new(ErrorCode::OptionInvalid, format!("There is no saved setup \u{201c}{}\u{201d}.", name.trim()))
        })
    }

    /// Checks and describes a setup or pass step, which is planned alone.
    fn check_alone(
        &self,
        game: &GameInstall,
        mods: &[vmm_core::backend::ModEntry],
        order: &[String],
        change: &Change,
    ) -> Result<PlanStep, VmmError> {
        let setups = self.fresh_setups()?;
        let no_pass =
            || VmmError::new(ErrorCode::PassState, "There is no compatibility pass under way. Start one first.");
        let quoted = |s: &str| format!("\u{201c}{}\u{201d}", s.trim());
        let list = |ids: &[String]| if ids.is_empty() { "none".to_string() } else { ids.join(", ") };
        let step = |description: String, changes_something: bool| PlanStep { description, changes_something };
        Ok(match change {
            Change::UseSetup { name } => {
                let s = &setups.saved[self.setup_index(name)?];
                let usable = |id: &String| mods.iter().any(|m| m.id == *id && m.problem.is_none());
                let (kept, gone): (Vec<String>, Vec<String>) = s.setup.order.iter().cloned().partition(usable);
                // Switching writes the setup's options, and a mod with none
                // saved or stored gets its defaults written, as turning it on
                // does: either way config.json changes.
                let now = self.current_setup(game)?;
                let stored = self.backend().stored_options(game)?;
                let mut changed = now.order != kept;
                for id in &kept {
                    changed |= match s.setup.options.get(id) {
                        Some(o) => stored.get(id) != Some(o),
                        None => self.backend().enable_defaults(game, id, &kept)?.is_some(),
                    };
                }
                let left_out = if gone.is_empty() {
                    String::new()
                } else {
                    format!(" (left out, no longer installed or readable: {})", gone.join(", "))
                };
                step(
                    format!(
                        "Switch to the setup {}: these mods on, in this order: {}{left_out}",
                        quoted(&s.name),
                        list(&kept)
                    ),
                    changed,
                )
            }
            Change::SaveSetup { name } => {
                if name.trim().is_empty() {
                    return Err(VmmError::new(ErrorCode::OptionInvalid, "Give the setup a name. Nothing was changed."));
                }
                let replacing = setups.saved.iter().any(|s| s.name.eq_ignore_ascii_case(name.trim()));
                let how = if replacing { ", replacing the saved one" } else { "" };
                step(
                    format!("Save the mods that are on now ({}) as the setup {}{how}", list(order), quoted(name)),
                    true,
                )
            }
            Change::DeleteSetup { name } => {
                let s = &setups.saved[self.setup_index(name)?];
                step(format!("Delete the saved setup {}", quoted(&s.name)), true)
            }
            Change::StartPass { from_current } => {
                if setups.pass.is_some() {
                    return Err(VmmError::new(
                        ErrorCode::PassState,
                        "A compatibility pass is already under way. End it first. Nothing was changed.",
                    ));
                }
                if *from_current {
                    step(
                        format!("Start a compatibility pass with the mods on now as its baseline: {}", list(order)),
                        true,
                    )
                } else {
                    step(format!("Start a compatibility pass from every mod off (turns off: {})", list(order)), true)
                }
            }
            Change::TryBatch { name, mods: batch } => {
                let mut pass = setups.pass.clone().ok_or_else(no_pass)?;
                for id in batch {
                    let m = mods.iter().find(|m| m.id == *id).ok_or_else(|| no_such_mod(id))?;
                    if let Some(problem) = &m.problem {
                        return Err(VmmError::new(
                            ErrorCode::ModUnreadable,
                            format!("{} can't be tried. {problem}", mod_label(m)),
                        ));
                    }
                }
                pass.begin(name, batch.clone())?;
                let target = pass.target_order(mods);
                let label = pass.batches.last().map(|b| b.name.clone()).unwrap_or_default();
                step(
                    format!(
                        "Try the batch {}: {} on top of the baseline; the load order becomes: {}",
                        quoted(&label),
                        list(batch),
                        list(&target)
                    ),
                    true,
                )
            }
            Change::MarkBatch { good, notes } => {
                let mut pass = setups.pass.clone().ok_or_else(no_pass)?;
                let i = pass.active().ok_or_else(|| {
                    VmmError::new(ErrorCode::PassState, "No batch is being tried. Nothing was changed.")
                })?;
                let label = pass.batches[i].name.clone();
                pass.mark(*good, order, notes)?;
                let target = pass.target_order(mods);
                if *good {
                    step(
                        format!(
                            "Mark the batch {} good: the baseline becomes: {}",
                            quoted(&label),
                            list(&pass.baseline)
                        ),
                        true,
                    )
                } else {
                    step(
                        format!(
                            "Mark the batch {} bad: the game goes back to the baseline: {}",
                            quoted(&label),
                            list(&target)
                        ),
                        target != order,
                    )
                }
            }
            Change::EndPass {} => {
                setups.pass.as_ref().ok_or_else(no_pass)?;
                step("End the compatibility pass and keep the mods as they are".into(), true)
            }
            other => unreachable!("{other:?} is checked in the plan's own loop"),
        })
    }

    /// The files these changes depend on.
    fn preconditions(&self, game: &GameInstall, changes: &[Change]) -> Result<Vec<Precondition>, VmmError> {
        // The mods whose options or defaults the plan resolves.
        let mut mods: Vec<String> = changes
            .iter()
            .filter_map(|c| match c {
                Change::SetEnabled { mod_id, enabled: true } | Change::SetOptions { mod_id, .. } => {
                    Some(mod_id.clone())
                }
                _ => None,
            })
            .collect();
        if let Some(c) = changes.iter().find(|c| c.stands_alone()) {
            let setups = self.fresh_setups()?;
            match c {
                Change::UseSetup { name } => {
                    if let Some(s) = setups.saved.iter().find(|s| s.name.eq_ignore_ascii_case(name.trim())) {
                        mods.extend(s.setup.order.iter().cloned());
                    }
                }
                Change::TryBatch { mods: batch, .. } => {
                    mods.extend(batch.iter().cloned());
                    if let Some(p) = &setups.pass {
                        mods.extend(p.baseline.iter().cloned());
                    }
                }
                Change::MarkBatch { .. } => {
                    if let Some(p) = &setups.pass {
                        mods.extend(p.baseline.iter().cloned());
                    }
                }
                _ => {}
            }
        }
        mods.sort();
        mods.dedup();
        let mut files = self.backend().state_files(game, &mods);
        if changes.iter().any(Change::stands_alone)
            && let Some(dir) = self.data_dir()
        {
            files.push(setups::path(dir));
        }
        if changes.iter().any(|c| matches!(c, Change::SetKind { .. }))
            && let Some(dir) = self.data_dir()
        {
            files.push(settings::path(dir));
        }
        files.into_iter().map(|p| Ok(Precondition { sha256: sha256_of(&p)?, path: p.display().to_string() })).collect()
    }

    /// Checks `changes` and saves them as a plan to apply with
    /// [`Engine::apply_plan`]. Changes nothing in the game.
    pub fn plan(&self, game: &GameInstall, changes: Vec<Change>) -> Result<Plan, VmmError> {
        let r = self.plan_inner(game, changes);
        match &r {
            Ok(p) => self.log_plan(
                LogLevel::Info,
                "plan.made",
                Some(game),
                None,
                Some(&p.plan_id),
                format!("Planned {} step(s).", p.steps.len()),
                None,
            ),
            Err(e) => self.log(LogLevel::Error, "plan.made", Some(game), None, e.message.clone(), Some(e)),
        }
        r
    }

    fn plan_inner(&self, game: &GameInstall, changes: Vec<Change>) -> Result<Plan, VmmError> {
        let dir = self.plans_dir()?;
        let steps = self.check(game, &changes)?;
        let preconditions = self.preconditions(game, &changes)?;
        let now = deploy::now_ms();
        let seed = format!("{now}:{}:{:?}:{:?}", std::process::id(), std::time::Instant::now(), changes);
        let plan_id = hex(seed.as_bytes())[..16].to_string();
        let plan = Plan {
            schema_version: SCHEMA_VERSION,
            plan_id,
            game: game.root.display().to_string(),
            changes,
            steps,
            preconditions,
            created_at_ms: now,
            expires_at_ms: now + PLAN_LIFETIME_MS,
        };
        std::fs::create_dir_all(&dir)
            .map_err(|e| VmmError::new(ErrorCode::SaveFailed, format!("Couldn't create {}: {e}", dir.display())))?;
        prune_expired(&dir, now);
        let path = dir.join(format!("{}.json", plan.plan_id));
        let tmp = dir.join(format!("{}.json.tmp", plan.plan_id));
        let json = serde_json::to_vec_pretty(&plan).expect("a plan serializes");
        std::fs::write(&tmp, json)
            .and_then(|()| std::fs::rename(&tmp, &path))
            .map_err(|e| VmmError::new(ErrorCode::SaveFailed, format!("Couldn't save the plan: {e}")))?;
        Ok(plan)
    }

    /// The game a saved plan is for: the install found at its folder, or
    /// that folder as it is. Lets `vmm apply` work without `--game`.
    pub fn plan_game(&self, plan_id: &str) -> Result<GameInstall, VmmError> {
        let not_found = || {
            VmmError::new(
                ErrorCode::PlanNotFound,
                format!("There is no plan \u{201c}{plan_id}\u{201d}. It may have been applied already."),
            )
        };
        if !valid_plan_id(plan_id) {
            return Err(not_found());
        }
        let path = self.plans_dir()?.join(format!("{plan_id}.json"));
        let plan: Plan =
            std::fs::read(&path).ok().and_then(|b| serde_json::from_slice(&b).ok()).ok_or_else(not_found)?;
        let root = PathBuf::from(&plan.game);
        Ok(self
            .installs()
            .into_iter()
            .find(|g| g.root == root)
            .unwrap_or(GameInstall { store: vmm_core::game::Store::Folder, root }))
    }

    /// Applies a saved plan, once, if nothing it depends on has changed, and
    /// records it in the journal with who asked. An AI tool (`Caller::Mcp`)
    /// must pass the user's `request` it acts on, and is refused while
    /// Settings' "Let AI tools make changes" is off.
    pub fn apply_plan(
        &self,
        game: &GameInstall,
        plan_id: &str,
        caller: &Caller,
        request: Option<&str>,
    ) -> Result<ApplyReport, VmmError> {
        let r = self.apply_plan_inner(game, plan_id, caller, request);
        let id = Some(plan_id).filter(|id| valid_plan_id(id));
        match &r {
            Ok(rep) => {
                let changed = rep.steps.iter().filter(|s| s.changes_something).count();
                let asked = request.map(|q| format!(" Asked for: \u{201c}{}\u{201d}", q.trim())).unwrap_or_default();
                self.log_plan(
                    LogLevel::Info,
                    "plan.applied",
                    Some(game),
                    None,
                    id,
                    format!("Applied a plan: {changed} of {} step(s) changed something.{asked}", rep.steps.len()),
                    None,
                )
            }
            Err(e) => self.log_plan(LogLevel::Error, "plan.applied", Some(game), None, id, e.message.clone(), Some(e)),
        }
        r
    }

    fn apply_plan_inner(
        &self,
        game: &GameInstall,
        plan_id: &str,
        caller: &Caller,
        request: Option<&str>,
    ) -> Result<ApplyReport, VmmError> {
        let request = request.map(str::trim).filter(|r| !r.is_empty());
        if caller.is_ai() {
            if !self.ai_changes_allowed() {
                return Err(VmmError::new(
                    ErrorCode::AiChangesOff,
                    "Changes from AI tools are turned off in Vaexil Mod Manager's Settings. Nothing was changed.",
                ));
            }
            if request.is_none() {
                return Err(VmmError::new(
                    ErrorCode::Usage,
                    "Pass the user's own request for this change as `request`. AI tools change nothing on their \
                     own initiative. Nothing was changed.",
                ));
            }
        }
        let not_found = || {
            VmmError::new(
                ErrorCode::PlanNotFound,
                format!("There is no plan \u{201c}{plan_id}\u{201d}. It may have been applied already."),
            )
        };
        if !valid_plan_id(plan_id) {
            return Err(not_found());
        }
        let dir = self.plans_dir()?;
        let path = dir.join(format!("{plan_id}.json"));
        let bytes = match std::fs::read(&path) {
            Ok(b) => b,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Err(not_found()),
            Err(e) => return Err(VmmError::new(ErrorCode::Io, format!("Couldn't read {}: {e}", path.display()))),
        };
        let plan: Plan = serde_json::from_slice(&bytes).map_err(|e| {
            VmmError::new(ErrorCode::SettingsUnreadable, format!("The plan file {} can't be read: {e}", path.display()))
        })?;
        if deploy::now_ms() > plan.expires_at_ms {
            let _ = std::fs::remove_file(&path);
            return Err(VmmError::new(
                ErrorCode::PlanExpired,
                "That plan is more than 10 minutes old. Nothing was changed.",
            ));
        }
        if Path::new(&plan.game) != game.root {
            return Err(VmmError::new(
                ErrorCode::GameNotFound,
                format!(
                    "That plan is for the game in {}, not {}. Nothing was changed.",
                    plan.game,
                    game.root.display()
                ),
            ));
        }
        if self.deploy_progress().0 || !host::running(&["Deploy.exe"]).is_empty() {
            return Err(VmmError::new(
                ErrorCode::DeployRunning,
                "SMF's Deploy.exe is running. Wait for it to finish; nothing was changed.",
            ));
        }
        for p in &plan.preconditions {
            if sha256_of(Path::new(&p.path))? != p.sha256 {
                return Err(VmmError::new(
                    ErrorCode::ConfigChangedMeanwhile,
                    format!("{} has changed since the plan was made. Nothing was changed.", p.path),
                ));
            }
        }
        // The files are as planned; check the changes once more against the
        // mods as they are now (a mod folder can go without touching them),
        // and refuse if they would now do anything other than the plan said.
        if self.check(game, &plan.changes)? != plan.steps {
            return Err(VmmError::new(
                ErrorCode::ConfigChangedMeanwhile,
                "What this plan would do has changed since it was made. Nothing was changed.",
            ));
        }
        std::fs::remove_file(&path).map_err(|e| {
            VmmError::new(ErrorCode::SaveFailed, format!("Couldn't mark the plan as used ({e}). Nothing was changed."))
        })?;

        let mut steps = Vec::new();
        // Whether the game needs a deploy follows from whether the backend's
        // settings file actually changed, whatever the steps were.
        let config_before = self.config_hash(game);
        for (i, change) in plan.changes.iter().enumerate() {
            let result = match change {
                Change::SetEnabled { mod_id, enabled } => self.set_enabled(game, mod_id, *enabled),
                Change::SetOptions { mod_id, chosen } => self.set_options(game, mod_id, chosen),
                Change::SetKind { mod_id, kind } => {
                    let before = self.kind_overrides().get(mod_id).copied();
                    self.set_kind(mod_id, *kind).map(|()| before != *kind)
                }
                Change::Reorder { order } => self.set_load_order(game, order),
                Change::AutoSort {} => {
                    let mods = self.inspect(game).mods;
                    let order = enabled_order(&mods);
                    vmm_core::order::auto_sort(&order, &mods)
                        .map_err(|stuck| {
                            VmmError::new(
                                ErrorCode::OrderMismatch,
                                format!("These mods' load rules contradict each other: {}.", stuck.join(", ")),
                            )
                        })
                        .and_then(|sorted| self.set_load_order(game, &sorted))
                }
                Change::UseSetup { name } => {
                    let before = self.config_hash(game);
                    self.setup_index(name)
                        .and_then(|i| self.use_setup(game, i))
                        .map(|_| self.config_hash(game) != before)
                }
                Change::SaveSetup { name } => self.save_setup(game, name).map(|()| true),
                Change::DeleteSetup { name } => {
                    self.setup_index(name).and_then(|i| self.delete_setup(i)).map(|()| true)
                }
                Change::StartPass { from_current } => self.start_pass(game, *from_current).map(|()| true),
                Change::TryBatch { name, mods } => self.begin_batch(game, name, mods.clone()).map(|()| true),
                Change::MarkBatch { good, notes } => {
                    let before = self.config_hash(game);
                    self.mark_batch(game, *good, notes).map(|()| *good || self.config_hash(game) != before)
                }
                Change::EndPass {} => self.end_pass().map(|()| true),
            };
            match result {
                Ok(changed) => {
                    steps.push(PlanStep { description: plan.steps[i].description.clone(), changes_something: changed });
                }
                Err(e) if i == 0 => return Err(e),
                Err(e) => {
                    // The steps before stay applied: record them, and who
                    // asked, with the failure.
                    let journal = self
                        .data_dir()
                        .map(|dir| journal::append(dir, caller, &plan.game, &plan.plan_id, request, &steps, Some(&e)));
                    let unrecorded = match journal {
                        Some(Err(j)) => format!(" They couldn't be added to the journal: {j}"),
                        _ => String::new(),
                    };
                    return Err(VmmError::new(
                        e.code,
                        format!(
                            "{} The first {i} step(s) of the plan were applied, each after a backup; the rest were \
                             not.{unrecorded}",
                            e.message
                        ),
                    ));
                }
            }
        }
        let needs_deploy = self.backend().capabilities().needs_deploy && self.config_hash(game) != config_before;
        let journal_problem = self.data_dir().and_then(|dir| {
            journal::append(dir, caller, &plan.game, &plan.plan_id, request, &steps, None)
                .err()
                .map(|e| format!("The change was made, but it couldn't be added to the journal: {e}"))
        });
        Ok(ApplyReport { schema_version: SCHEMA_VERSION, plan_id: plan.plan_id, steps, needs_deploy, journal_problem })
    }
}

impl Engine {
    /// The saved setups and the pass under way, for `vmm setups` and AI tools.
    pub fn setup_list(&self, game: Option<&GameInstall>) -> Result<SetupList, VmmError> {
        let saved = self.fresh_setups()?;
        let now = game.and_then(|g| self.current_setup(g).ok());
        let mods = game.map(|g| self.inspect(g).mods).unwrap_or_default();
        let state = |s: setups::BatchStatus| match s {
            setups::BatchStatus::Testing => BatchState::Testing,
            setups::BatchStatus::Good => BatchState::Good,
            setups::BatchStatus::Bad => BatchState::Bad,
        };
        Ok(SetupList {
            schema_version: SCHEMA_VERSION,
            setups: saved
                .saved
                .iter()
                .map(|s| SetupView {
                    name: s.name.clone(),
                    order: s.setup.order.clone(),
                    options: s.setup.options.clone(),
                    in_use: now.as_ref().is_some_and(|n| n.order == s.setup.order),
                })
                .collect(),
            pass: saved.pass.as_ref().map(|p| PassView {
                baseline: p.baseline.clone(),
                batches: p
                    .batches
                    .iter()
                    .map(|b| BatchView {
                        name: b.name.clone(),
                        mods: b.mods.clone(),
                        state: state(b.status),
                        notes: b.notes.clone(),
                    })
                    .collect(),
                candidates: p.candidates(&mods).into_iter().map(|m| m.id.clone()).collect(),
            }),
        })
    }

    /// Starts a deploy for a front end other than the window. An AI tool must
    /// pass the user's request, and is refused while Settings' "Let AI tools
    /// make changes" is off: a deploy rewrites the game's patch files.
    pub fn start_deploy_as(&self, game: &GameInstall, caller: &Caller, request: Option<&str>) -> Result<(), VmmError> {
        let request = request.map(str::trim).filter(|r| !r.is_empty());
        if caller.is_ai() {
            if !self.ai_changes_allowed() {
                return Err(VmmError::new(
                    ErrorCode::AiChangesOff,
                    "Changes from AI tools are turned off in Vaexil Mod Manager's Settings. Nothing was deployed.",
                ));
            }
            if request.is_none() {
                return Err(VmmError::new(
                    ErrorCode::Usage,
                    "Pass the user's own request for this deploy as `request`. Nothing was deployed.",
                ));
            }
        }
        self.start_deploy(game, false)
    }

    /// Where a deploy started by [`Engine::start_deploy_as`] has got to. Once
    /// it has finished, records it (as the window does) and includes its
    /// report.
    pub fn deploy_status(&self, game: Option<&GameInstall>) -> DeployStatus {
        let (running, last_line) = self.deploy_progress();
        let finished = !running && self.finish_deploy(game).is_some();
        DeployStatus {
            schema_version: SCHEMA_VERSION,
            running,
            last_line,
            report: if finished || !running { self.deploy_report() } else { None },
        }
    }

    /// Deploys and waits for the end, for `vmm deploy`. `on_line` gets the
    /// deploy tool's latest line as it changes.
    pub fn deploy_and_wait(&self, game: &GameInstall, on_line: &mut dyn FnMut(&str)) -> Result<DeployStatus, VmmError> {
        self.start_deploy_as(game, &Caller::Cli, None)?;
        let mut last = String::new();
        loop {
            std::thread::sleep(std::time::Duration::from_millis(250));
            let (running, line) = self.deploy_progress();
            if line != last {
                on_line(&line);
                last = line;
            }
            if !running {
                if let Some(Some(problem)) = self.finish_deploy(Some(game)) {
                    return Err(problem);
                }
                return Ok(DeployStatus {
                    schema_version: SCHEMA_VERSION,
                    running: false,
                    last_line: last,
                    report: self.deploy_report(),
                });
            }
        }
    }
}

fn no_such_mod(id: &str) -> VmmError {
    VmmError::new(ErrorCode::ModNotFound, format!("No installed mod has the id \u{201c}{id}\u{201d}."))
}

/// Deletes plans that can no longer be applied. Best effort.
fn prune_expired(dir: &Path, now: u64) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for e in entries.flatten() {
        let path = e.path();
        let expired = std::fs::read(&path)
            .ok()
            .and_then(|b| serde_json::from_slice::<Plan>(&b).ok())
            .is_some_and(|p| now > p.expires_at_ms);
        if expired {
            let _ = std::fs::remove_file(path);
        }
    }
}

#[cfg(test)]
mod tests {
    use vmm_core::game::Store;

    use super::*;

    fn fake_game(root: &Path) -> GameInstall {
        crate::tests::fake_game(root)
    }

    fn setup() -> (tempfile::TempDir, GameInstall, Engine) {
        let tmp = tempfile::tempdir().unwrap();
        let game = fake_game(&tmp.path().join("game"));
        let engine = Engine::with_data_dir(Some(tmp.path().join("data")));
        (tmp, game, engine)
    }

    #[test]
    fn reads_mods_order_and_status() {
        let (_tmp, game, engine) = setup();
        let list = engine.mod_list(&game).unwrap();
        assert_eq!(
            list.mods.iter().map(|m| (m.id.as_str(), m.enabled)).collect::<Vec<_>>(),
            [("A", true), ("B", false), ("C", false)]
        );
        assert_eq!(engine.mod_detail(&game, "C").unwrap().mod_view.options[0].value, "X");
        // The list counts options; the detail has them.
        let c = list.mods.iter().find(|m| m.id == "C").unwrap();
        assert_eq!(c.option_count, engine.mod_detail(&game, "C").unwrap().mod_view.options.len());
        assert_eq!(engine.mod_detail(&game, "Z").unwrap_err().code, ErrorCode::ModNotFound);
        let order = engine.order_report(&game).unwrap();
        assert_eq!((order.order_name.as_str(), order.order.clone()), ("smf", vec!["A".to_string()]));
        let status = engine.status(Some(&game));
        assert_eq!((status.installed, status.enabled), (3, 1));
        assert_eq!(status.backend.unwrap().state, BackendState::Installed);
    }

    #[test]
    fn a_plan_is_checked_applied_once_and_refused_when_stale() {
        let (_tmp, game, engine) = setup();
        let plan = engine
            .plan(
                &game,
                vec![
                    Change::SetEnabled { mod_id: "C".into(), enabled: true },
                    Change::SetOptions { mod_id: "C".into(), chosen: vec![] },
                    Change::Reorder { order: vec!["C".into(), "A".into()] },
                    Change::SetKind { mod_id: "A".into(), kind: Some(ModKind::Sound) },
                ],
            )
            .unwrap();
        assert!(plan.steps.iter().all(|s| s.changes_something), "{:?}", plan.steps);
        assert_eq!(plan.preconditions.len(), 3, "config.json, C's manifest and settings.json");
        let report = engine.apply_plan(&game, &plan.plan_id, &Caller::Cli, None).unwrap();
        assert!(report.needs_deploy);
        assert_eq!(engine.current_setup(&game).unwrap().order, ["C", "A"]);
        assert_eq!(engine.kind_overrides().get("A"), Some(&ModKind::Sound));
        assert_eq!(
            engine.apply_plan(&game, &plan.plan_id, &Caller::Cli, None).unwrap_err().code,
            ErrorCode::PlanNotFound
        );

        // A change made after planning makes the plan stale.
        let plan = engine.plan(&game, vec![Change::SetEnabled { mod_id: "B".into(), enabled: true }]).unwrap();
        engine.set_enabled(&game, "A", false).unwrap();
        assert_eq!(
            engine.apply_plan(&game, &plan.plan_id, &Caller::Cli, None).unwrap_err().code,
            ErrorCode::ConfigChangedMeanwhile
        );
        assert!(!engine.current_setup(&game).unwrap().order.contains(&"B".to_string()));
    }

    #[test]
    fn plans_refuse_what_cannot_be_done() {
        let (_tmp, game, engine) = setup();
        let code = |changes: Vec<Change>| engine.plan(&game, changes).unwrap_err().code;
        assert_eq!(code(vec![]), ErrorCode::Usage);
        assert_eq!(code(vec![Change::SetEnabled { mod_id: "Z".into(), enabled: true }]), ErrorCode::ModNotFound);
        assert_eq!(
            code(vec![Change::SetOptions { mod_id: "C".into(), chosen: vec!["Y".into()] }]),
            ErrorCode::OptionInvalid
        );
        // B is off at that point, so it can't be in the order.
        assert_eq!(code(vec![Change::Reorder { order: vec!["A".into(), "B".into()] }]), ErrorCode::OrderMismatch);
        assert_eq!(code(vec![Change::Reorder { order: vec!["A".into(), "A".into()] }]), ErrorCode::OrderMismatch);
        // Turned on earlier in the same plan, it can be.
        engine
            .plan(
                &game,
                vec![
                    Change::SetEnabled { mod_id: "B".into(), enabled: true },
                    Change::Reorder { order: vec!["B".into(), "A".into()] },
                ],
            )
            .unwrap();
    }

    /// Adds mod D, whose option needs A, and mod E, with a select group.
    fn with_option_mods(game: &GameInstall) {
        let mods = game.root.join("Simple Mod Framework").join("Mods");
        for (id, options) in [
            ("D", "[{ name: 'Y', type: 'checkbox', requirements: ['A'] }]"),
            (
                "E",
                "[{ name: 'P', type: 'select', group: 'G' }, { name: 'Q', type: 'select', group: 'G', enabledByDefault: true }]",
            ),
        ] {
            std::fs::create_dir_all(mods.join(id)).unwrap();
            std::fs::write(
                mods.join(id).join("manifest.json"),
                format!("{{ id: '{id}', name: '{id}', options: {options} }}"),
            )
            .unwrap();
        }
    }

    #[test]
    fn option_steps_meet_the_state_the_plan_leaves() {
        let (_tmp, game, engine) = setup();
        with_option_mods(&game);
        let y = || Change::SetOptions { mod_id: "D".into(), chosen: vec!["Y".into()] };
        // A is on now, so Y is fine on its own...
        engine.plan(&game, vec![y()]).unwrap();
        // ...but not after the same plan has turned A off.
        let err = engine.plan(&game, vec![Change::SetEnabled { mod_id: "A".into(), enabled: false }, y()]).unwrap_err();
        assert_eq!(err.code, ErrorCode::OptionRequirementOff);
        // An empty choice for a select group is described as what applying
        // writes: the group's default.
        let plan = engine.plan(&game, vec![Change::SetOptions { mod_id: "E".into(), chosen: vec![] }]).unwrap();
        assert!(plan.steps[0].description.ends_with(": G:Q"), "{}", plan.steps[0].description);
    }

    #[test]
    fn a_changed_manifest_or_unreadable_settings_stop_a_plan_before_any_write() {
        let (tmp, game, engine) = setup();
        with_option_mods(&game);
        // E's default is Q. A plan for no explicit choice resolves to G:Q...
        let plan = engine.plan(&game, vec![Change::SetOptions { mod_id: "E".into(), chosen: vec![] }]).unwrap();
        assert_eq!(plan.preconditions.len(), 2, "config.json and E's manifest");
        // ...and if the author makes P the default meanwhile, it isn't applied.
        let manifest = game.root.join("Simple Mod Framework/Mods/E/manifest.json");
        std::fs::write(
            &manifest,
            "{ id: 'E', name: 'E', options: [{ name: 'P', type: 'select', group: 'G', enabledByDefault: true }, \
             { name: 'Q', type: 'select', group: 'G' }] }",
        )
        .unwrap();
        let err = engine.apply_plan(&game, &plan.plan_id, &Caller::Cli, None).unwrap_err();
        assert_eq!(err.code, ErrorCode::ConfigChangedMeanwhile);

        // A type change with unreadable settings is refused when planning.
        std::fs::create_dir_all(tmp.path().join("data")).unwrap();
        std::fs::write(tmp.path().join("data/settings.json"), "{ not json").unwrap();
        let err = engine
            .plan(
                &game,
                vec![
                    Change::SetEnabled { mod_id: "B".into(), enabled: true },
                    Change::SetKind { mod_id: "A".into(), kind: Some(ModKind::Sound) },
                ],
            )
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::SettingsUnreadable);
    }

    #[test]
    fn turning_on_a_mod_that_is_on_but_unreadable_is_refused_when_planning() {
        let (_tmp, game, engine) = setup();
        // A is on; its manifest becomes unreadable.
        std::fs::write(game.root.join("Simple Mod Framework/Mods/A/manifest.json"), "{ not json").unwrap();
        let err = engine
            .plan(
                &game,
                vec![
                    Change::SetEnabled { mod_id: "B".into(), enabled: true },
                    Change::SetEnabled { mod_id: "A".into(), enabled: true },
                ],
            )
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::ModUnreadable);
        // Turning it off is still allowed.
        engine.plan(&game, vec![Change::SetEnabled { mod_id: "A".into(), enabled: false }]).unwrap();
    }

    #[test]
    fn an_enable_step_names_its_place_and_a_rule_change_stales_the_plan() {
        let (_tmp, game, engine) = setup();
        let plan = engine.plan(&game, vec![Change::SetEnabled { mod_id: "B".into(), enabled: true }]).unwrap();
        assert!(
            plan.steps[0].description.ends_with("at place 2 of 2 in the load order"),
            "{}",
            plan.steps[0].description
        );
        assert_eq!(engine.plan_game(&plan.plan_id).unwrap().root, game.root);
        // A (on) now says it must load after B, so B would go first instead.
        std::fs::write(
            game.root.join("Simple Mod Framework/Mods/A/manifest.json"),
            "{ id: 'A', name: 'A mod', loadAfter: ['B'] }",
        )
        .unwrap();
        let err = engine.apply_plan(&game, &plan.plan_id, &Caller::Cli, None).unwrap_err();
        assert_eq!(err.code, ErrorCode::ConfigChangedMeanwhile);
    }

    #[test]
    fn an_option_chosen_earlier_in_a_plan_brings_its_load_rules() {
        let (_tmp, game, engine) = setup();
        // A (on) has an option that makes it load after B.
        std::fs::write(
            game.root.join("Simple Mod Framework/Mods/A/manifest.json"),
            "{ id: 'A', name: 'A mod', options: [{ name: 'X', type: 'checkbox', loadAfter: ['B'] }] }",
        )
        .unwrap();
        let plan = engine
            .plan(
                &game,
                vec![
                    Change::SetOptions { mod_id: "A".into(), chosen: vec!["X".into()] },
                    Change::SetEnabled { mod_id: "B".into(), enabled: true },
                ],
            )
            .unwrap();
        assert!(
            plan.steps[1].description.ends_with("at place 1 of 2 in the load order"),
            "{}",
            plan.steps[1].description
        );
        engine.apply_plan(&game, &plan.plan_id, &Caller::Cli, None).unwrap();
        assert_eq!(engine.current_setup(&game).unwrap().order, ["B", "A"]);
    }

    #[test]
    fn a_default_option_that_an_earlier_step_makes_usable_brings_its_rules() {
        let (_tmp, game, engine) = setup();
        // A (on, no options stored) has a default option that needs B and
        // makes A load after C. Turning B on makes that option apply.
        std::fs::write(
            game.root.join("Simple Mod Framework/Mods/A/manifest.json"),
            "{ id: 'A', name: 'A mod', options: [{ name: 'X', type: 'checkbox', enabledByDefault: true, \
             requirements: ['B'], loadAfter: ['C'] }] }",
        )
        .unwrap();
        let plan = engine
            .plan(
                &game,
                vec![
                    Change::SetEnabled { mod_id: "B".into(), enabled: true },
                    Change::SetEnabled { mod_id: "C".into(), enabled: true },
                ],
            )
            .unwrap();
        let placed = engine.current_setup(&game).unwrap().order;
        assert_eq!(placed, ["A"], "planning changed nothing");
        engine.apply_plan(&game, &plan.plan_id, &Caller::Cli, None).unwrap();
        let order = engine.current_setup(&game).unwrap().order;
        let c = order.iter().position(|m| m == "C").unwrap();
        let described: usize = plan.steps[1]
            .description
            .split("at place ")
            .nth(1)
            .and_then(|r| r.split(' ').next())
            .and_then(|n| n.parse().ok())
            .unwrap();
        assert_eq!(c + 1, described, "{order:?} vs {}", plan.steps[1].description);
    }

    #[test]
    fn defaults_stored_by_an_earlier_enable_stay_for_later_steps() {
        let (_tmp, game, engine) = setup();
        // B (off, no options stored) has a default needing A that makes B
        // load after C. Enabling B while A is on stores it; it stays after A
        // is turned off.
        std::fs::write(
            game.root.join("Simple Mod Framework/Mods/B/manifest.json"),
            "{ id: 'B', name: 'B mod', options: [{ name: 'X', type: 'checkbox', enabledByDefault: true, \
             requirements: ['A'], loadAfter: ['C'] }] }",
        )
        .unwrap();
        let plan = engine
            .plan(
                &game,
                vec![
                    Change::SetEnabled { mod_id: "B".into(), enabled: true },
                    Change::SetEnabled { mod_id: "A".into(), enabled: false },
                    Change::SetEnabled { mod_id: "C".into(), enabled: true },
                ],
            )
            .unwrap();
        engine.apply_plan(&game, &plan.plan_id, &Caller::Cli, None).unwrap();
        let order = engine.current_setup(&game).unwrap().order;
        let described: usize = plan.steps[2]
            .description
            .split("at place ")
            .nth(1)
            .and_then(|r| r.split(' ').next())
            .and_then(|n| n.parse().ok())
            .unwrap();
        assert_eq!(order.iter().position(|m| m == "C").unwrap() + 1, described, "{order:?}");
        assert_eq!(order, ["C", "B"]);
    }

    #[test]
    fn storing_the_shown_defaults_is_a_change() {
        let (_tmp, game, engine) = setup();
        // A is on with an option shown chosen by default, and nothing stored.
        std::fs::write(
            game.root.join("Simple Mod Framework/Mods/A/manifest.json"),
            "{ id: 'A', name: 'A mod', options: [{ name: 'X', type: 'checkbox', enabledByDefault: true }] }",
        )
        .unwrap();
        assert!(engine.mod_detail(&game, "A").unwrap().mod_view.options[0].chosen);
        let set = || Change::SetOptions { mod_id: "A".into(), chosen: vec!["X".into()] };
        let plan = engine.plan(&game, vec![set()]).unwrap();
        assert!(plan.steps[0].changes_something, "storing it changes what Deploy.exe applies");
        assert!(engine.apply_plan(&game, &plan.plan_id, &Caller::Cli, None).unwrap().steps[0].changes_something);
        // Stored now, the same choice is already so.
        assert!(!engine.plan(&game, vec![set()]).unwrap().steps[0].changes_something);
    }

    #[test]
    fn a_relative_game_folder_is_made_absolute() {
        let (tmp, _game, engine) = setup();
        let cwd = std::env::current_dir().unwrap();
        let relative = pathdiff(&tmp.path().join("game"), &cwd);
        let game = engine.pick_game(Some(&relative)).unwrap();
        assert!(game.root.is_absolute(), "{}", game.root.display());
    }

    /// `to` relative to `from` when they share a drive; otherwise `to`.
    fn pathdiff(to: &Path, from: &Path) -> PathBuf {
        let (to_parts, from_parts): (Vec<_>, Vec<_>) = (to.components().collect(), from.components().collect());
        if to_parts.first() != from_parts.first() {
            return to.to_path_buf();
        }
        let common = to_parts.iter().zip(&from_parts).take_while(|(a, b)| a == b).count();
        let mut out = PathBuf::new();
        for _ in common..from_parts.len() {
            out.push("..");
        }
        for c in &to_parts[common..] {
            out.push(c.as_os_str());
        }
        out
    }

    /// Plans and applies one change from the CLI; returns the step.
    fn run(engine: &Engine, game: &GameInstall, change: Change) -> PlanStep {
        let plan = engine.plan(game, vec![change]).unwrap();
        engine.apply_plan(game, &plan.plan_id, &Caller::Cli, None).unwrap();
        plan.steps[0].clone()
    }

    #[test]
    fn a_compatibility_pass_runs_through_plans() {
        let (_tmp, game, engine) = setup();
        let order = |e: &Engine| e.current_setup(&game).unwrap().order;
        let step = run(&engine, &game, Change::StartPass { from_current: false });
        assert!(step.description.contains("from every mod off (turns off: A)"), "{}", step.description);
        assert!(order(&engine).is_empty());
        // A second pass can't start while one is under way.
        let err = engine.plan(&game, vec![Change::StartPass { from_current: true }]).unwrap_err();
        assert_eq!(err.code, ErrorCode::PassState);

        let step = run(&engine, &game, Change::TryBatch { name: "First".into(), mods: vec!["A".into(), "B".into()] });
        assert!(step.description.contains("\u{201c}First\u{201d}: A, B"), "{}", step.description);
        assert_eq!(order(&engine), ["A", "B"]);
        let step = run(&engine, &game, Change::MarkBatch { good: false, notes: "crashes".into() });
        assert!(step.description.contains("bad: the game goes back to the baseline: none"), "{}", step.description);
        assert!(order(&engine).is_empty());

        run(&engine, &game, Change::TryBatch { name: String::new(), mods: vec!["C".into()] });
        let step = run(&engine, &game, Change::MarkBatch { good: true, notes: String::new() });
        assert!(step.description.ends_with("good: the baseline becomes: C"), "{}", step.description);
        assert_eq!(order(&engine), ["C"]);
        run(&engine, &game, Change::EndPass {});
        assert!(Engine::with_data_dir(engine.data_dir().map(Path::to_path_buf)).setups().unwrap().pass.is_none());
        assert_eq!(order(&engine), ["C"]);
    }

    #[test]
    fn setups_are_saved_used_and_deleted_through_plans_one_step_at_a_time() {
        let (_tmp, game, engine) = setup();
        run(&engine, &game, Change::SaveSetup { name: "Just A".into() });
        run(&engine, &game, Change::SetEnabled { mod_id: "B".into(), enabled: true });
        // Setup steps stand alone.
        let err = engine
            .plan(
                &game,
                vec![
                    Change::SetEnabled { mod_id: "C".into(), enabled: true },
                    Change::UseSetup { name: "Just A".into() },
                ],
            )
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::Usage);
        let step = run(&engine, &game, Change::UseSetup { name: "just a".into() });
        assert!(step.changes_something && step.description.ends_with("in this order: A"), "{}", step.description);
        assert_eq!(engine.current_setup(&game).unwrap().order, ["A"]);
        run(&engine, &game, Change::DeleteSetup { name: "Just A".into() });
        let err = engine.plan(&game, vec![Change::UseSetup { name: "Just A".into() }]).unwrap_err();
        assert_eq!(err.code, ErrorCode::OptionInvalid);
    }

    #[test]
    fn auto_sort_keeps_the_rules() {
        let (_tmp, game, engine) = setup();
        std::fs::write(
            game.root.join("Simple Mod Framework/Mods/A/manifest.json"),
            "{ id: 'A', name: 'A mod', loadAfter: ['B'] }",
        )
        .unwrap();
        run(&engine, &game, Change::SetEnabled { mod_id: "B".into(), enabled: true });
        // B went where A's rule wants it; put it last by hand, then sort.
        run(&engine, &game, Change::Reorder { order: vec!["A".into(), "B".into()] });
        let step = run(&engine, &game, Change::AutoSort {});
        assert!(step.changes_something && step.description.ends_with(": B, A"), "{}", step.description);
        assert_eq!(engine.current_setup(&game).unwrap().order, ["B", "A"]);
    }

    #[test]
    fn the_journal_keeps_a_failure_beside_the_steps_applied() {
        let tmp = tempfile::tempdir().unwrap();
        let step = PlanStep { description: "Turn B on".into(), changes_something: true };
        let e = VmmError::new(ErrorCode::SaveFailed, "disk full");
        journal::append(tmp.path(), &Caller::Cli, "g", "0123456789abcdef", None, std::slice::from_ref(&step), None)
            .unwrap();
        journal::append(
            tmp.path(),
            &Caller::Mcp { client: "x".into() },
            "g",
            "0123456789abcdef",
            Some("why"),
            &[step],
            Some(&e),
        )
        .unwrap();
        let entries = journal::read(tmp.path());
        assert!(entries[0].get("failed").is_none());
        assert_eq!(entries[1]["failed"]["code"], "save.failed");
        assert_eq!(
            (entries[1]["steps"][0]["description"].as_str(), entries[1]["request"].as_str()),
            (Some("Turn B on"), Some("why"))
        );
    }

    #[test]
    fn setup_steps_act_on_setups_saved_by_another_front_end() {
        let (tmp, game, engine) = setup();
        let data = tmp.path().join("data");
        run(&engine, &game, Change::SaveSetup { name: "One".into() });
        // Another front end replaces "One" with B alone and adds "Two".
        let other = Engine::with_data_dir(Some(data.clone()));
        other.set_enabled(&game, "A", false).unwrap();
        other.set_enabled(&game, "B", true).unwrap();
        other.save_setup(&game, "One").unwrap();
        other.save_setup(&game, "Two").unwrap();
        other.set_enabled(&game, "B", false).unwrap();
        // This engine, made before, uses the new "One" and keeps "Two".
        let step = run(&engine, &game, Change::UseSetup { name: "One".into() });
        assert!(step.description.ends_with("in this order: B"), "{}", step.description);
        assert_eq!(engine.current_setup(&game).unwrap().order, ["B"]);
        run(&engine, &game, Change::SaveSetup { name: "Three".into() });
        let names: Vec<String> = engine.setups().unwrap().saved.into_iter().map(|s| s.name).collect();
        assert_eq!(names, ["One", "Two", "Three"]);
    }

    #[test]
    fn switching_to_a_setup_that_fills_in_defaults_is_a_change() {
        let (_tmp, game, engine) = setup();
        // A is on with an option it has never had stored; the setup is A alone.
        std::fs::write(
            game.root.join("Simple Mod Framework/Mods/A/manifest.json"),
            "{ id: 'A', name: 'A mod', options: [{ name: 'X', type: 'checkbox', enabledByDefault: true }] }",
        )
        .unwrap();
        let mut saved = engine.setups().unwrap();
        saved.put("Bare", vmm_core::backend::Setup { order: vec!["A".into()], options: Default::default() }).unwrap();
        saved.save(engine.data_dir().unwrap()).unwrap();
        let plan = engine.plan(&game, vec![Change::UseSetup { name: "Bare".into() }]).unwrap();
        assert!(plan.steps[0].changes_something, "the defaults get stored");
        let report = engine.apply_plan(&game, &plan.plan_id, &Caller::Cli, None).unwrap();
        assert!(report.steps[0].changes_something && report.needs_deploy);
        // Now stored, switching again changes nothing.
        let plan = engine.plan(&game, vec![Change::UseSetup { name: "Bare".into() }]).unwrap();
        assert!(!plan.steps[0].changes_something);
    }

    #[test]
    fn marking_a_batch_good_needs_no_deploy() {
        let (_tmp, game, engine) = setup();
        let apply = |c: Change| {
            let plan = engine.plan(&game, vec![c]).unwrap();
            engine.apply_plan(&game, &plan.plan_id, &Caller::Cli, None).unwrap()
        };
        apply(Change::StartPass { from_current: false });
        assert!(apply(Change::TryBatch { name: String::new(), mods: vec!["B".into()] }).needs_deploy);
        let good = apply(Change::MarkBatch { good: true, notes: String::new() });
        assert!(good.steps[0].changes_something, "the pass is saved");
        assert!(!good.needs_deploy, "the game is as it was tried");
        assert!(!apply(Change::SaveSetup { name: "S".into() }).needs_deploy);
    }

    #[test]
    fn plan_ids_are_never_paths() {
        let (_tmp, game, engine) = setup();
        for id in ["../settings", "0123456789ABCDEF", "", "0123456789abcdef0"] {
            assert_eq!(
                engine.apply_plan(&game, id, &Caller::Cli, None).unwrap_err().code,
                ErrorCode::PlanNotFound,
                "{id}"
            );
        }
        let other = GameInstall { store: Store::Folder, root: PathBuf::from("C:\\elsewhere") };
        let plan = engine.plan(&game, vec![Change::SetEnabled { mod_id: "B".into(), enabled: true }]).unwrap();
        assert_eq!(
            engine.apply_plan(&other, &plan.plan_id, &Caller::Cli, None).unwrap_err().code,
            ErrorCode::GameNotFound
        );
    }

    #[test]
    fn an_expired_plan_is_refused_and_removed() {
        let (tmp, game, engine) = setup();
        let plan = engine.plan(&game, vec![Change::SetEnabled { mod_id: "B".into(), enabled: true }]).unwrap();
        let path = tmp.path().join("data").join(PLANS).join(format!("{}.json", plan.plan_id));
        let mut old = plan.clone();
        old.expires_at_ms = 1;
        std::fs::write(&path, serde_json::to_vec(&old).unwrap()).unwrap();
        assert_eq!(
            engine.apply_plan(&game, &plan.plan_id, &Caller::Cli, None).unwrap_err().code,
            ErrorCode::PlanExpired
        );
        assert!(!path.exists());
    }
}
