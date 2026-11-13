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
    ApplyReport, BackendState, BackendView, BrokenRuleView, Change, DeployState, GameList, GameView, KindSource,
    ModDetail, ModList, ModView, OptionView, OrderReport, Plan, PlanStep, Precondition, RelationView, SCHEMA_VERSION,
    StatusReport,
};

use crate::{Engine, deploy, enabled_order, host, settings};

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
    chosen: BTreeMap<String, BTreeSet<String>>,
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
            return Ok(GameInstall { store: vmm_core::game::Store::Folder, root: folder.to_path_buf() });
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

    pub fn mod_list(&self, game: &GameInstall) -> Result<ModList, VmmError> {
        let inspection = self.readable(game)?;
        let kinds = self.kind_overrides();
        let replaced = vmm_core::order::overridden_by(&enabled_order(&inspection.mods), &inspection.mods);
        Ok(ModList {
            schema_version: SCHEMA_VERSION,
            game: game_view(game),
            mods: inspection.mods.iter().map(|m| mod_view(m, &kinds, &replaced)).collect(),
        })
    }

    pub fn mod_detail(&self, game: &GameInstall, mod_id: &str) -> Result<ModDetail, VmmError> {
        let list = self.mod_list(game)?;
        let m = list.mods.into_iter().find(|m| m.id == mod_id).ok_or_else(|| no_such_mod(mod_id))?;
        Ok(ModDetail { schema_version: SCHEMA_VERSION, game: list.game, mod_view: m })
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
        let inspection = self.readable(game)?;
        let mods = &inspection.mods;
        let find = |id: &str| mods.iter().find(|m| m.id == id).ok_or_else(|| no_such_mod(id));
        let mut sim = Simulated {
            order: enabled_order(mods),
            chosen: mods
                .iter()
                .map(|m| (m.id.clone(), m.options.iter().filter(|o| o.chosen).map(|o| o.value.clone()).collect()))
                .collect(),
            kinds: self.kind_overrides(),
        };
        let mut steps = Vec::new();
        for change in changes {
            let step = match change {
                Change::SetEnabled { mod_id, enabled } => {
                    let m = find(mod_id)?;
                    let on = sim.order.contains(mod_id);
                    if *enabled && !on {
                        if let Some(problem) = &m.problem {
                            return Err(VmmError::new(
                                ErrorCode::ModUnreadable,
                                format!("{} can't be turned on. {problem}", mod_label(m)),
                            ));
                        }
                        sim.order.push(mod_id.clone());
                        // Turning a mod on gives it its default options.
                        let defaults = m.options.iter().filter(|o| o.chosen).map(|o| o.value.clone()).collect();
                        sim.chosen.insert(mod_id.clone(), defaults);
                    } else if !*enabled {
                        sim.order.retain(|id| id != mod_id);
                    }
                    PlanStep {
                        description: format!("Turn {} {}", mod_label(m), if *enabled { "on" } else { "off" }),
                        changes_something: *enabled != on,
                    }
                }
                Change::SetOptions { mod_id, chosen } => {
                    let m = find(mod_id)?;
                    let mut groups = BTreeSet::new();
                    for value in chosen {
                        let o = m.options.iter().find(|o| &o.value == value).ok_or_else(|| {
                            VmmError::new(
                                ErrorCode::OptionInvalid,
                                format!("{} has no option \u{201c}{value}\u{201d}. Nothing was changed.", mod_label(m)),
                            )
                        })?;
                        if let Some(g) = &o.group
                            && !groups.insert(g.clone())
                        {
                            return Err(VmmError::new(
                                ErrorCode::OptionInvalid,
                                format!("Only one choice of \u{201c}{g}\u{201d} can be chosen. Nothing was changed."),
                            ));
                        }
                        let off: Vec<&String> = o.needs.iter().filter(|n| !sim.order.contains(n)).collect();
                        if !off.is_empty() {
                            return Err(VmmError::new(
                                ErrorCode::OptionRequirementOff,
                                format!(
                                    "\u{201c}{}\u{201d} needs {} turned on first. Nothing was changed.",
                                    o.label,
                                    off.iter().map(|s| s.as_str()).collect::<Vec<_>>().join(", ")
                                ),
                            ));
                        }
                    }
                    let new: BTreeSet<String> = chosen.iter().cloned().collect();
                    let changed = sim.chosen.get(mod_id) != Some(&new);
                    sim.chosen.insert(mod_id.clone(), new);
                    let list = if chosen.is_empty() { "none".to_string() } else { chosen.join(", ") };
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
            };
            steps.push(step);
        }
        Ok(steps)
    }

    /// The files these changes depend on.
    fn preconditions(&self, game: &GameInstall, changes: &[Change]) -> Result<Vec<Precondition>, VmmError> {
        let mut files = self.backend().state_files(game);
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

    /// Applies a saved plan, once, if nothing it depends on has changed.
    pub fn apply_plan(&self, game: &GameInstall, plan_id: &str) -> Result<ApplyReport, VmmError> {
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
        // mods as they are now (a mod folder can go without touching them).
        self.check(game, &plan.changes)?;
        std::fs::remove_file(&path).map_err(|e| {
            VmmError::new(ErrorCode::SaveFailed, format!("Couldn't mark the plan as used ({e}). Nothing was changed."))
        })?;

        let mut steps = Vec::new();
        let mut needs_deploy = false;
        for (i, change) in plan.changes.iter().enumerate() {
            let result = match change {
                Change::SetEnabled { mod_id, enabled } => self.set_enabled(game, mod_id, *enabled),
                Change::SetOptions { mod_id, chosen } => self.set_options(game, mod_id, chosen),
                Change::SetKind { mod_id, kind } => {
                    let before = self.kind_overrides().get(mod_id).copied();
                    self.set_kind(mod_id, *kind).map(|()| before != *kind)
                }
                Change::Reorder { order } => self.set_load_order(game, order),
            };
            match result {
                Ok(changed) => {
                    needs_deploy |= changed && !matches!(change, Change::SetKind { .. });
                    steps.push(PlanStep { description: plan.steps[i].description.clone(), changes_something: changed });
                }
                Err(e) if i == 0 => return Err(e),
                Err(e) => {
                    return Err(VmmError::new(
                        e.code,
                        format!(
                            "{} The first {i} step(s) of the plan were applied, each after a backup; the rest were not.",
                            e.message
                        ),
                    ));
                }
            }
        }
        needs_deploy &= self.backend().capabilities().needs_deploy;
        Ok(ApplyReport { schema_version: SCHEMA_VERSION, plan_id: plan.plan_id, steps, needs_deploy })
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
        assert_eq!(plan.preconditions.len(), 2);
        let report = engine.apply_plan(&game, &plan.plan_id).unwrap();
        assert!(report.needs_deploy);
        assert_eq!(engine.current_setup(&game).unwrap().order, ["C", "A"]);
        assert_eq!(engine.kind_overrides().get("A"), Some(&ModKind::Sound));
        assert_eq!(engine.apply_plan(&game, &plan.plan_id).unwrap_err().code, ErrorCode::PlanNotFound);

        // A change made after planning makes the plan stale.
        let plan = engine.plan(&game, vec![Change::SetEnabled { mod_id: "B".into(), enabled: true }]).unwrap();
        engine.set_enabled(&game, "A", false).unwrap();
        assert_eq!(engine.apply_plan(&game, &plan.plan_id).unwrap_err().code, ErrorCode::ConfigChangedMeanwhile);
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

    #[test]
    fn plan_ids_are_never_paths() {
        let (_tmp, game, engine) = setup();
        for id in ["../settings", "0123456789ABCDEF", "", "0123456789abcdef0"] {
            assert_eq!(engine.apply_plan(&game, id).unwrap_err().code, ErrorCode::PlanNotFound, "{id}");
        }
        let other = GameInstall { store: Store::Folder, root: PathBuf::from("C:\\elsewhere") };
        let plan = engine.plan(&game, vec![Change::SetEnabled { mod_id: "B".into(), enabled: true }]).unwrap();
        assert_eq!(engine.apply_plan(&other, &plan.plan_id).unwrap_err().code, ErrorCode::GameNotFound);
    }

    #[test]
    fn an_expired_plan_is_refused_and_removed() {
        let (tmp, game, engine) = setup();
        let plan = engine.plan(&game, vec![Change::SetEnabled { mod_id: "B".into(), enabled: true }]).unwrap();
        let path = tmp.path().join("data").join(PLANS).join(format!("{}.json", plan.plan_id));
        let mut old = plan.clone();
        old.expires_at_ms = 1;
        std::fs::write(&path, serde_json::to_vec(&old).unwrap()).unwrap();
        assert_eq!(engine.apply_plan(&game, &plan.plan_id).unwrap_err().code, ErrorCode::PlanExpired);
        assert!(!path.exists());
    }
}
