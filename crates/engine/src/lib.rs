//! The headless engine of Vaexil Mod Manager (ADR 0003). Every front end (the
//! window today; the `vmm` command line and the MCP server next) does
//! everything through [`Engine`], so each operation is written once and works
//! the same way everywhere.
//!
//! The engine picks the backend (SMF, the only one so far), finds the game,
//! keeps the app's own files (settings, setups, the last deploy) in its data
//! folder, and runs deploys. Every change it makes to the game's mod
//! framework goes through the backend, after a backup.

pub mod contract;
pub mod crash;
pub mod deploy;
pub mod diagnose;
pub mod events;
pub mod host;
pub mod http;
pub mod journal;
pub mod settings;
pub mod setups;
pub mod smf_update;
pub mod steam_play;
pub mod store;

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::path::PathBuf;

use vmm_backend_smf::Smf;
use vmm_core::backend::{Backend, Inspection, ModKind, Setup};
use vmm_core::error::{ErrorCode, VmmError};
use vmm_core::game::GameInstall;
use vmm_core::schema::{ErrorReport, LogEvent, LogLevel};

use journal::Caller;
use settings::Settings;
use setups::{Pass, Setups};

/// What the engine needs from its surroundings, so tests can run it against
/// throwaway folders.
pub struct Engine {
    backend: Box<dyn Backend>,
    /// `%LOCALAPPDATA%\Vaexil Mod Manager`, or `None` when Windows doesn't say
    /// where that is; then nothing that needs a backup is allowed.
    data_dir: Option<PathBuf>,
    settings: RefCell<Result<Settings, VmmError>>,
    setups: RefCell<Result<Setups, VmmError>>,
    record: RefCell<deploy::Record>,
    progress: deploy::Shared,
    /// Who is using this engine, for the event log and the journal.
    caller: Caller,
}

impl Engine {
    /// The engine for this PC: the SMF backend, with backups in the app's
    /// data folder.
    pub fn open() -> Engine {
        Engine::open_as(Caller::Window)
    }

    /// The engine for this PC, used by `caller`. Old event logs are pruned.
    pub fn open_as(caller: Caller) -> Engine {
        let mut engine = Engine::with_data_dir(host::data_dir());
        engine.caller = caller;
        if let Some(dir) = engine.logs_dir() {
            events::prune(&dir, deploy::now_ms());
        }
        engine
    }

    pub fn caller(&self) -> &Caller {
        &self.caller
    }

    /// Where the event logs and deploy logs go.
    pub fn logs_dir(&self) -> Option<PathBuf> {
        self.data_dir.as_ref().map(|d| d.join("logs"))
    }

    /// Appends an event to the log. Best effort.
    pub(crate) fn log(
        &self,
        level: LogLevel,
        event: &str,
        game: Option<&GameInstall>,
        mod_id: Option<&str>,
        message: String,
        error: Option<&VmmError>,
    ) {
        self.log_plan(level, event, game, mod_id, None, message, error);
    }

    /// As [`Engine::log`], for an event that is part of a plan.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn log_plan(
        &self,
        level: LogLevel,
        event: &str,
        game: Option<&GameInstall>,
        mod_id: Option<&str>,
        plan: Option<&str>,
        message: String,
        error: Option<&VmmError>,
    ) {
        let Some(dir) = self.logs_dir() else { return };
        let ev = LogEvent {
            ts_ms: deploy::now_ms(),
            level,
            event: event.into(),
            caller: self.caller.label(),
            game: game.map(|g| g.root.display().to_string()),
            backend: Some(self.backend.id().into()),
            mod_id: mod_id.map(str::to_string),
            plan: plan.map(str::to_string),
            message,
            error: error.map(|e| ErrorReport::from(e).error),
        };
        let _ = events::append(&dir, &ev);
    }

    /// Logs the outcome of a change: `ok` says what changed (or `None` when
    /// nothing did, which isn't logged); a failure is logged as an error.
    fn noted<T>(
        &self,
        event: &str,
        game: Option<&GameInstall>,
        mod_id: Option<&str>,
        result: Result<T, VmmError>,
        ok: impl FnOnce(&T) -> Option<String>,
    ) -> Result<T, VmmError> {
        match &result {
            Ok(v) => {
                if let Some(message) = ok(v) {
                    self.log(LogLevel::Info, event, game, mod_id, message, None);
                }
            }
            Err(e) => self.log(LogLevel::Error, event, game, mod_id, e.message.clone(), Some(e)),
        }
        result
    }

    /// The newest `limit` events, newest last.
    pub fn recent_events(&self, limit: usize) -> Vec<LogEvent> {
        self.logs_dir().map(|d| events::recent(&d, limit)).unwrap_or_default()
    }

    pub fn with_data_dir(data_dir: Option<PathBuf>) -> Engine {
        let backend: Box<dyn Backend> = match &data_dir {
            Some(dir) => {
                Box::new(Smf::with_backups(dir.join("config-backups")).with_learned(dir.join(smf_update::LEARNED)))
            }
            None => Box::new(Smf::default()),
        };
        let settings = match &data_dir {
            Some(dir) => Settings::load(dir),
            None => Ok(Settings::default()),
        };
        let setups = match &data_dir {
            Some(dir) => Setups::load(dir),
            None => Ok(Setups::default()),
        };
        let record = data_dir.as_deref().map(deploy::Record::load).unwrap_or_default();
        Engine {
            backend,
            data_dir,
            settings: RefCell::new(settings),
            setups: RefCell::new(setups),
            record: RefCell::new(record),
            progress: deploy::Shared::default(),
            caller: Caller::Window,
        }
    }

    pub fn backend(&self) -> &dyn Backend {
        self.backend.as_ref()
    }

    pub fn data_dir(&self) -> Option<&std::path::Path> {
        self.data_dir.as_deref()
    }

    /// Every install of the game found, or the one `VAEXIL_MOD_MANAGER_GAME`
    /// names.
    pub fn installs(&self) -> Vec<GameInstall> {
        match host::game_override() {
            Some(game) => vec![game],
            None => vmm_core::game::find_installs(&host::sources()),
        }
    }

    pub fn inspect(&self, game: &GameInstall) -> Inspection {
        self.backend.inspect(game)
    }

    fn need_backups(&self) -> Result<(), VmmError> {
        if self.data_dir.is_none() {
            return Err(VmmError::new(
                ErrorCode::BackupUnavailable,
                "Windows didn't say where your local app data folder is (%LOCALAPPDATA%), so there is nowhere to \
                 keep a copy of SMF's config.json. Nothing was changed.",
            ));
        }
        Ok(())
    }

    // Mods.

    pub fn set_enabled(&self, game: &GameInstall, mod_id: &str, on: bool) -> Result<bool, VmmError> {
        let r = self.need_backups().and_then(|()| self.backend.set_enabled(game, mod_id, on));
        let (event, word) = if on { ("mod.enabled", "on") } else { ("mod.disabled", "off") };
        self.noted(event, Some(game), Some(mod_id), r, |changed| changed.then(|| format!("Turned {mod_id} {word}.")))
    }

    pub fn set_options(&self, game: &GameInstall, mod_id: &str, chosen: &[String]) -> Result<bool, VmmError> {
        let r = self.need_backups().and_then(|()| self.backend.set_options(game, mod_id, chosen));
        self.noted("mod.options", Some(game), Some(mod_id), r, |changed| {
            changed.then(|| format!("Chose options of {mod_id}: {}.", chosen.join(", ")))
        })
    }

    pub fn set_load_order(&self, game: &GameInstall, order: &[String]) -> Result<bool, VmmError> {
        let r = self.need_backups().and_then(|()| self.backend.set_load_order(game, order));
        self.noted("order.saved", Some(game), None, r, |changed| {
            changed.then(|| format!("Saved the load order: {}.", order.join(", ")))
        })
    }

    // The app's own settings.

    /// The types the user chose, by mod id, read again from disk so a type
    /// saved by another window or the CLI shows too.
    pub fn kind_overrides(&self) -> BTreeMap<String, ModKind> {
        if let Some(dir) = &self.data_dir {
            *self.settings.borrow_mut() = Settings::load(dir);
        }
        self.settings.borrow().as_ref().map(|s| s.kind_overrides.clone()).unwrap_or_default()
    }

    /// Why the app's settings can't be used, if they can't.
    pub fn settings_problem(&self) -> Option<String> {
        if self.data_dir.is_none() {
            return Some("Windows didn't say where your local app data folder is, so types can't be changed.".into());
        }
        self.settings.borrow().as_ref().err().map(|e| e.message.clone())
    }

    /// Sets (or, with `None`, clears) the type the user chose for a mod. The
    /// settings are read again first, so another window's change survives.
    pub fn set_kind(&self, mod_id: &str, kind: Option<ModKind>) -> Result<(), VmmError> {
        let r = self.set_kind_inner(mod_id, kind);
        let what = kind.map_or("the type its files suggest", |k| k.label());
        self.noted("mod.kind", None, Some(mod_id), r, |()| Some(format!("Showed {mod_id} as {what}.")))
    }

    fn set_kind_inner(&self, mod_id: &str, kind: Option<ModKind>) -> Result<(), VmmError> {
        let dir = self.data_dir.clone().ok_or_else(|| {
            VmmError::new(
                ErrorCode::BackupUnavailable,
                "There is no folder to keep the type in. The type was not changed.",
            )
        })?;
        let mut settings = Settings::load(&dir)?;
        match kind {
            Some(k) => settings.kind_overrides.insert(mod_id.to_string(), k),
            None => settings.kind_overrides.remove(mod_id),
        };
        settings.save(&dir)?;
        *self.settings.borrow_mut() = Ok(settings);
        Ok(())
    }

    /// Settings' "Let AI tools make changes", read again from disk so a
    /// change in the window reaches a running `vmm mcp` at once.
    pub fn ai_changes_allowed(&self) -> bool {
        self.data_dir.as_deref().and_then(|d| Settings::load(d).ok()).is_some_and(|s| s.ai_changes)
    }

    pub fn set_ai_changes(&self, on: bool) -> Result<(), VmmError> {
        let r = self.set_ai_changes_inner(on);
        let word = if on { "allowed" } else { "turned off" };
        self.noted("settings.ai_changes", None, None, r, |()| Some(format!("Changes from AI tools {word}.")))
    }

    fn set_ai_changes_inner(&self, on: bool) -> Result<(), VmmError> {
        let dir = self.data_dir.clone().ok_or_else(|| {
            VmmError::new(ErrorCode::BackupUnavailable, "There is no folder to keep settings in. Nothing was changed.")
        })?;
        let mut settings = Settings::load(&dir)?;
        settings.ai_changes = on;
        settings.save(&dir)?;
        *self.settings.borrow_mut() = Ok(settings);
        Ok(())
    }

    // Setups.

    /// The saved setups and the pass, read again from disk, so a change
    /// made by another front end (the window, `vmm`, an AI tool) is never
    /// overwritten or acted on from a stale copy.
    pub fn setups(&self) -> Result<Setups, VmmError> {
        if let Some(dir) = &self.data_dir {
            *self.setups.borrow_mut() = Setups::load(dir);
        }
        self.setups.borrow().clone()
    }

    fn save_setups(&self, setups: Setups) -> Result<(), VmmError> {
        let dir = self
            .data_dir
            .as_ref()
            .ok_or_else(|| VmmError::new(ErrorCode::BackupUnavailable, "There is no folder to keep setups in."))?;
        setups.save(dir)?;
        *self.setups.borrow_mut() = Ok(setups);
        Ok(())
    }

    pub fn current_setup(&self, game: &GameInstall) -> Result<Setup, VmmError> {
        self.backend.current_setup(game)
    }

    /// Saves the mods that are on now as a setup named `name`.
    pub fn save_setup(&self, game: &GameInstall, name: &str) -> Result<(), VmmError> {
        let r = self.setups().and_then(|mut setups| {
            setups.put(name, self.backend.current_setup(game)?)?;
            self.save_setups(setups)
        });
        self.noted("setup.saved", Some(game), None, r, |()| Some(format!("Saved the setup \u{201c}{name}\u{201d}.")))
    }

    /// Switches to the saved setup at `index`. Returns its name and the mods
    /// it names that are no longer installed.
    pub fn use_setup(&self, game: &GameInstall, index: usize) -> Result<(String, Vec<String>), VmmError> {
        let r = self.need_backups().and_then(|()| {
            let setups = self.setups()?;
            let s = setups
                .saved
                .get(index)
                .ok_or_else(|| VmmError::new(ErrorCode::OptionInvalid, "That setup is gone."))?;
            let skipped = self.backend.apply_setup(game, &s.setup)?;
            Ok((s.name.clone(), skipped))
        });
        self.noted("setup.used", Some(game), None, r, |(name, _)| {
            Some(format!("Switched to the setup \u{201c}{name}\u{201d}."))
        })
    }

    pub fn delete_setup(&self, index: usize) -> Result<(), VmmError> {
        let r = self.setups().and_then(|mut setups| {
            let name = (index < setups.saved.len()).then(|| setups.saved.remove(index).name);
            self.save_setups(setups).map(|()| name)
        });
        self.noted("setup.deleted", None, None, r, |name| {
            name.as_ref().map(|n| format!("Deleted the setup \u{201c}{n}\u{201d}."))
        })
        .map(|_| ())
    }

    // Compatibility passes.

    /// Runs `step` on the pass, puts the game in the pass's target order, and
    /// saves the pass. A step that fails changes nothing.
    fn pass_step(
        &self,
        game: &GameInstall,
        step: impl FnOnce(&mut Option<Pass>, &[vmm_core::backend::ModEntry]) -> Result<(), VmmError>,
    ) -> Result<(), VmmError> {
        self.need_backups()?;
        let mods = self.backend.inspect(game).mods;
        let mut setups = self.setups()?;
        step(&mut setups.pass, &mods)?;
        if let Some(pass) = &setups.pass {
            let target = pass.target_order(&mods);
            self.backend.apply_setup(game, &Setup { order: target, options: Default::default() })?;
        }
        self.save_setups(setups)
    }

    /// Starts a pass: from every mod off, or from the mods on now.
    pub fn start_pass(&self, game: &GameInstall, from_current: bool) -> Result<(), VmmError> {
        let r = self.pass_step(game, |pass, mods| {
            let baseline = if from_current { enabled_order(mods) } else { vec![] };
            *pass = Some(Pass { baseline, batches: vec![] });
            Ok(())
        });
        let from = if from_current { "the mods on now" } else { "every mod off" };
        self.noted("pass.started", Some(game), None, r, |()| Some(format!("Started a compatibility pass from {from}.")))
    }

    pub fn begin_batch(&self, game: &GameInstall, name: &str, mods: Vec<String>) -> Result<(), VmmError> {
        let list = mods.join(", ");
        let r = self.pass_step(game, |pass, _| {
            pass.as_mut().ok_or_else(|| VmmError::new(ErrorCode::PassState, "Start a pass first."))?.begin(name, mods)
        });
        self.noted("pass.batch", Some(game), None, r, |()| Some(format!("Trying a batch: {list}.")))
    }

    pub fn mark_batch(&self, game: &GameInstall, good: bool, notes: &str) -> Result<(), VmmError> {
        let r = self.pass_step(game, |pass, mods| {
            let order = enabled_order(mods);
            pass.as_mut()
                .ok_or_else(|| VmmError::new(ErrorCode::PassState, "There is no pass."))?
                .mark(good, &order, notes)
        });
        let word = if good { "good" } else { "bad" };
        self.noted("pass.marked", Some(game), None, r, |()| Some(format!("Marked the batch {word}. {notes}")))
    }

    /// Ends the pass and keeps the game as it is now.
    pub fn end_pass(&self) -> Result<(), VmmError> {
        let r = self.setups().and_then(|mut setups| {
            setups.pass = None;
            self.save_setups(setups)
        });
        self.noted("pass.ended", None, None, r, |()| Some("Ended the compatibility pass.".into()))
    }

    // Deploy and Play.

    pub fn fingerprint(&self, game: &GameInstall) -> Result<String, VmmError> {
        self.backend.deploy_fingerprint(game)
    }

    /// A cheap stand-in for [`Engine::fingerprint`]'s cache: it changes with
    /// anything the app itself writes and with mods added or removed, but not
    /// with a file changed deep inside a mod.
    pub fn quick_key(&self, game: &GameInstall) -> String {
        vmm_backend_smf::find_folder(&game.root).map(|f| vmm_backend_smf::deploy::quick_key(&f)).unwrap_or_default()
    }

    /// The last finished deploy, read again from disk, so a deploy finished
    /// by another front end (the command line, an AI tool) shows too.
    pub fn record(&self) -> deploy::Record {
        if let Some(dir) = &self.data_dir {
            *self.record.borrow_mut() = deploy::Record::load(dir);
        }
        self.record.borrow().clone()
    }

    /// Shares deploy progress with other engines in this process (the MCP
    /// server makes an engine per call, and a deploy outlives the call).
    pub fn share_progress(&mut self, progress: deploy::Shared) {
        self.progress = progress;
    }

    /// Why a deploy or Play can't start now (the game, or another deploy, is
    /// running), if it can't.
    pub fn blocker(&self) -> Option<VmmError> {
        if self.data_dir.as_deref().is_some_and(smf_update::updating) {
            return Some(VmmError::new(ErrorCode::SmfUpdating, "SMF is being updated. Wait for it to finish."));
        }
        deploy::blocked(&host::running(&deploy::BLOCKERS))
    }

    /// Starts a deploy on a worker thread. `then_play` starts the game when it
    /// succeeds (see [`Engine::finish_deploy`]).
    pub fn start_deploy(&self, game: &GameInstall, then_play: bool) -> Result<(), VmmError> {
        let r = match self.blocker() {
            Some(why) => Err(why),
            // A stale copy that can't be set aside would make SMF build
            // from the old game build, so the deploy doesn't start.
            None => self.set_aside_stale_thumbs(game).map(|_| {
                deploy::start(&self.progress, game.clone(), self.logs_dir(), then_play);
            }),
        };
        self.noted("deploy.started", Some(game), None, r, |()| Some("Started a deploy.".into()))
    }

    /// Before a deploy: if SMF's `cleanThumbs.dat` is from an older game build
    /// than the game's own thumbs.dat (after a game update), keep a copy and
    /// remove it, so SMF copies the current one again, as it does when the
    /// file is missing. Returns what was done, if anything, or why a stale
    /// copy couldn't be set aside.
    pub fn set_aside_stale_thumbs(&self, game: &GameInstall) -> Result<Option<String>, VmmError> {
        let Some(folder) = vmm_backend_smf::find_folder(&game.root) else { return Ok(None) };
        let Some(stale) = vmm_backend_smf::thumbs::stale(&folder, &game.root) else { return Ok(None) };
        let Some(data) = self.data_dir() else {
            return Err(VmmError::new(
                ErrorCode::SaveFailed,
                "SMF's cleanThumbs.dat is from an older game build, and there is nowhere to keep a copy of it.",
            ));
        };
        let clean = folder.join("cleanThumbs.dat");
        let backups = data.join("smf-backups");
        let backup = backups.join(format!("cleanThumbs-{}-{:015}.dat", stale.clean, deploy::now_ms()));
        let r = std::fs::create_dir_all(&backups)
            .and_then(|()| std::fs::copy(&clean, &backup))
            .and_then(|_| std::fs::remove_file(&clean))
            .map_err(|e| {
                VmmError::new(ErrorCode::SaveFailed, format!("Couldn't set aside the stale cleanThumbs.dat: {e}"))
            });
        let message = format!(
            "SMF's cleanThumbs.dat was for game build {} and the game is now build {}, so it was set aside (copy in {}); SMF copies the current one again.",
            stale.clean,
            stale.game,
            backup.display()
        );
        self.noted("smf.clean_thumbs", Some(game), None, r, |()| Some(message.clone())).map(|()| Some(message))
    }

    /// Whether a deploy is running, and its latest output line.
    pub fn deploy_progress(&self) -> (bool, String) {
        let p = self.progress.lock().expect("deploy progress lock");
        (p.running, p.last_line.clone())
    }

    /// Takes a deploy that has just finished, records it, and (when it was
    /// started by Play and succeeded) starts the game. Returns what to tell
    /// the user, if anything went wrong; `None` when nothing finished.
    pub fn finish_deploy(&self, game: Option<&GameInstall>) -> Option<Option<VmmError>> {
        let f = self.progress.lock().expect("deploy progress lock").finished.take()?;
        let success = matches!(f.result, Ok(ref o) if o.success);
        let record = deploy::Record {
            fingerprint: if success { f.fingerprint.clone() } else { None },
            success: Some(success),
            exit_code: f.result.as_ref().ok().and_then(|o| o.exit_code),
            log: f.log.map(|p| p.display().to_string()),
            at_ms: Some(deploy::now_ms()),
            started_at_ms: Some(f.started_ms),
        };
        match &f.result {
            Ok(_) if success => {
                self.log(LogLevel::Info, "deploy.finished", game, None, "The deploy succeeded.".into(), None)
            }
            Ok(o) => self.log(
                LogLevel::Warn,
                "deploy.finished",
                game,
                None,
                format!("The deploy failed (exit code {}).", o.exit_code.map_or("none".into(), |c| c.to_string())),
                None,
            ),
            Err(e) => self.log(LogLevel::Error, "deploy.finished", game, None, e.message.clone(), Some(e)),
        }
        if let Some(dir) = &self.data_dir {
            let _ = record.save(dir);
        }
        *self.record.borrow_mut() = record;
        let problem = match &f.result {
            Err(e) => Some(VmmError::new(e.code, format!("Couldn't run SMF's Deploy.exe. {}", e.message))),
            Ok(_) if !success => None,
            Ok(_) if f.then_play => game.and_then(|g| deploy::launch(g).err()),
            Ok(_) => None,
        };
        Some(problem)
    }

    /// Whether the game has exactly the mods and settings of the last
    /// successful deploy. `None` when that can't be worked out.
    pub fn up_to_date(&self, game: &GameInstall) -> Option<bool> {
        let current = self.backend.deploy_fingerprint(game).ok()?;
        Some(self.record().fingerprint.as_deref() == Some(current.as_str()))
    }

    /// Starts the game as it is now, without deploying: the build the last
    /// deploy left in the game. Refused while a deploy is writing it.
    pub fn launch_as_is(&self, game: &GameInstall) -> Result<(), VmmError> {
        self.check_no_deploy()?;
        // When Steam's Play button starts this app, it would ask again: leave
        // word that the choice was made just now.
        if let (Some(dir), vmm_core::game::Store::Steam) = (&self.data_dir, game.store) {
            let _ = std::fs::write(dir.join(PLAY_AS_IS), deploy::now_ms().to_string());
        }
        let r = deploy::launch(game);
        self.noted("game.started", Some(game), None, r, |()| Some("Started HITMAN without deploying.".into()))
    }
}

/// Left by [`Engine::launch_as_is`] for the Steam handoff: the user chose to
/// play without deploying, and Steam's Play button shouldn't ask again.
const PLAY_AS_IS: &str = "play-as-is";
/// How long that choice stands.
const PLAY_AS_IS_MS: u64 = 2 * 60 * 1000;

impl Engine {
    /// Refuses while a deploy is writing the game's files.
    pub fn check_no_deploy(&self) -> Result<(), VmmError> {
        if self.data_dir.as_deref().is_some_and(smf_update::updating) {
            return Err(VmmError::new(
                ErrorCode::SmfUpdating,
                "SMF is being updated. Wait for it to finish, then play.",
            ));
        }
        if self.deploy_progress().0 || !host::running(&["Deploy.exe"]).is_empty() {
            return Err(VmmError::new(
                ErrorCode::DeployRunning,
                "SMF's Deploy.exe is running. Wait for it to finish, then play.",
            ));
        }
        Ok(())
    }

    /// Whether the user chose to play without deploying in the last two
    /// minutes (in the window, or `vmm play --no-deploy`). Taken once.
    pub fn take_play_as_is(&self) -> bool {
        let Some(path) = self.data_dir.as_ref().map(|d| d.join(PLAY_AS_IS)) else { return false };
        let at: Option<u64> = std::fs::read_to_string(&path).ok().and_then(|t| t.trim().parse().ok());
        let _ = std::fs::remove_file(&path);
        at.is_some_and(|t| deploy::now_ms().saturating_sub(t) < PLAY_AS_IS_MS)
    }
}

/// The ids of the mods that are on, first to last.
pub fn enabled_order(mods: &[vmm_core::backend::ModEntry]) -> Vec<String> {
    let mut on: Vec<&vmm_core::backend::ModEntry> = mods.iter().filter(|m| m.load_position.is_some()).collect();
    on.sort_by_key(|m| m.load_position);
    on.into_iter().map(|m| m.id.clone()).collect()
}

#[cfg(test)]
pub(crate) mod tests {
    use std::path::Path;

    use vmm_core::game::Store;

    use super::*;

    /// A fake game with SMF and three plain mods; the third has options.
    pub(crate) fn fake_game(root: &Path) -> GameInstall {
        let smf = root.join("Simple Mod Framework");
        std::fs::create_dir_all(root.join("Retail")).unwrap();
        std::fs::write(root.join("Retail/HITMAN3.exe"), b"").unwrap();
        std::fs::create_dir_all(&smf).unwrap();
        std::fs::write(smf.join("Deploy.exe"), b"").unwrap();
        std::fs::write(smf.join("config.json"), "{ loadOrder: ['A'] }").unwrap();
        for (folder, id, extra) in [
            ("A", "A", ""),
            ("B", "B", ""),
            ("C", "C", ", options: [{ name: 'X', type: 'checkbox', enabledByDefault: true }]"),
        ] {
            let dir = smf.join("Mods").join(folder);
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(dir.join("manifest.json"), format!("{{ id: '{id}', name: '{id} mod'{extra} }}")).unwrap();
        }
        GameInstall { store: Store::Folder, root: root.to_path_buf() }
    }

    #[test]
    fn changes_need_a_data_folder_for_backups() {
        let tmp = tempfile::tempdir().unwrap();
        let game = fake_game(tmp.path());
        let engine = Engine::with_data_dir(None);
        let err = engine.set_enabled(&game, "B", true).unwrap_err();
        assert_eq!(err.code, ErrorCode::BackupUnavailable);
        assert!(engine.set_kind("B", Some(ModKind::Sound)).is_err());
        assert!(engine.settings_problem().is_some());
    }

    #[test]
    fn mods_types_setups_and_a_pass_go_through_the_engine() {
        let tmp = tempfile::tempdir().unwrap();
        let game = fake_game(&tmp.path().join("game"));
        let data = tmp.path().join("data");
        let engine = Engine::with_data_dir(Some(data.clone()));
        let order = |e: &Engine| e.current_setup(&game).unwrap().order;

        assert!(engine.set_enabled(&game, "B", true).unwrap());
        assert_eq!(order(&engine), ["A", "B"]);
        assert!(data.join("config-backups").is_dir());

        engine.set_kind("B", Some(ModKind::Sound)).unwrap();
        assert_eq!(Engine::with_data_dir(Some(data.clone())).kind_overrides().get("B"), Some(&ModKind::Sound));
        engine.set_kind("B", None).unwrap();
        assert!(engine.kind_overrides().is_empty());

        engine.save_setup(&game, "Two").unwrap();
        engine.start_pass(&game, false).unwrap();
        assert!(order(&engine).is_empty());
        engine.begin_batch(&game, "", vec!["C".into()]).unwrap();
        assert_eq!(order(&engine), ["C"]);
        // A batch mod gets its default options.
        assert_eq!(engine.current_setup(&game).unwrap().options.get("C"), Some(&vec!["X".to_string()]));
        engine.mark_batch(&game, true, "fine").unwrap();
        assert_eq!(engine.setups().unwrap().pass.unwrap().baseline, ["C"]);
        engine.end_pass().unwrap();
        assert!(engine.setups().unwrap().pass.is_none());

        let (name, skipped) = engine.use_setup(&game, 0).unwrap();
        assert_eq!((name.as_str(), skipped.len()), ("Two", 0));
        assert_eq!(order(&engine), ["A", "B"]);
        engine.delete_setup(0).unwrap();
        assert!(Engine::with_data_dir(Some(data)).setups().unwrap().saved.is_empty());
    }

    #[test]
    fn a_choice_to_play_as_is_is_taken_once_and_expires() {
        let tmp = tempfile::tempdir().unwrap();
        let engine = Engine::with_data_dir(Some(tmp.path().to_path_buf()));
        assert!(!engine.take_play_as_is());
        std::fs::write(tmp.path().join(PLAY_AS_IS), deploy::now_ms().to_string()).unwrap();
        assert!(engine.take_play_as_is());
        assert!(!engine.take_play_as_is(), "used once");
        std::fs::write(tmp.path().join(PLAY_AS_IS), (deploy::now_ms() - PLAY_AS_IS_MS - 1).to_string()).unwrap();
        assert!(!engine.take_play_as_is(), "too old");
    }

    #[test]
    fn enabled_order_follows_load_positions() {
        let m = |id: &str, pos: Option<usize>| vmm_core::backend::ModEntry {
            id: id.into(),
            load_position: pos,
            ..Default::default()
        };
        assert_eq!(enabled_order(&[m("b", Some(2)), m("off", None), m("a", Some(1))]), ["a", "b"]);
    }
}
