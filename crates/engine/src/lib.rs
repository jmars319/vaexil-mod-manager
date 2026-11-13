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
pub mod deploy;
pub mod host;
pub mod settings;
pub mod setups;
pub mod store;

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::path::PathBuf;

use vmm_backend_smf::Smf;
use vmm_core::backend::{Backend, Inspection, ModKind, Setup};
use vmm_core::error::{ErrorCode, VmmError};
use vmm_core::game::GameInstall;

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
}

impl Engine {
    /// The engine for this PC: the SMF backend, with backups in the app's
    /// data folder.
    pub fn open() -> Engine {
        Engine::with_data_dir(host::data_dir())
    }

    pub fn with_data_dir(data_dir: Option<PathBuf>) -> Engine {
        let backend: Box<dyn Backend> = match &data_dir {
            Some(dir) => Box::new(Smf::with_backups(dir.join("config-backups"))),
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
        self.need_backups()?;
        self.backend.set_enabled(game, mod_id, on)
    }

    pub fn set_options(&self, game: &GameInstall, mod_id: &str, chosen: &[String]) -> Result<bool, VmmError> {
        self.need_backups()?;
        self.backend.set_options(game, mod_id, chosen)
    }

    pub fn set_load_order(&self, game: &GameInstall, order: &[String]) -> Result<bool, VmmError> {
        self.need_backups()?;
        self.backend.set_load_order(game, order)
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

    // Setups.

    pub fn setups(&self) -> Result<Setups, VmmError> {
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
        let mut setups = self.setups()?;
        setups.put(name, self.backend.current_setup(game)?)?;
        self.save_setups(setups)
    }

    /// Switches to the saved setup at `index`. Returns its name and the mods
    /// it names that are no longer installed.
    pub fn use_setup(&self, game: &GameInstall, index: usize) -> Result<(String, Vec<String>), VmmError> {
        self.need_backups()?;
        let setups = self.setups()?;
        let s =
            setups.saved.get(index).ok_or_else(|| VmmError::new(ErrorCode::OptionInvalid, "That setup is gone."))?;
        let skipped = self.backend.apply_setup(game, &s.setup)?;
        Ok((s.name.clone(), skipped))
    }

    pub fn delete_setup(&self, index: usize) -> Result<(), VmmError> {
        let mut setups = self.setups()?;
        if index < setups.saved.len() {
            setups.saved.remove(index);
        }
        self.save_setups(setups)
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
        self.pass_step(game, |pass, mods| {
            let baseline = if from_current { enabled_order(mods) } else { vec![] };
            *pass = Some(Pass { baseline, batches: vec![] });
            Ok(())
        })
    }

    pub fn begin_batch(&self, game: &GameInstall, name: &str, mods: Vec<String>) -> Result<(), VmmError> {
        self.pass_step(game, |pass, _| {
            pass.as_mut().ok_or_else(|| VmmError::new(ErrorCode::PassState, "Start a pass first."))?.begin(name, mods)
        })
    }

    pub fn mark_batch(&self, game: &GameInstall, good: bool, notes: &str) -> Result<(), VmmError> {
        self.pass_step(game, |pass, mods| {
            let order = enabled_order(mods);
            pass.as_mut()
                .ok_or_else(|| VmmError::new(ErrorCode::PassState, "There is no pass."))?
                .mark(good, &order, notes)
        })
    }

    /// Ends the pass and keeps the game as it is now.
    pub fn end_pass(&self) -> Result<(), VmmError> {
        let mut setups = self.setups()?;
        setups.pass = None;
        self.save_setups(setups)
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

    pub fn record(&self) -> deploy::Record {
        self.record.borrow().clone()
    }

    /// Why a deploy or Play can't start now (the game, or another deploy, is
    /// running), if it can't.
    pub fn blocker(&self) -> Option<VmmError> {
        deploy::blocked(&host::running(&deploy::BLOCKERS))
    }

    /// Starts a deploy on a worker thread. `then_play` starts the game when it
    /// succeeds (see [`Engine::finish_deploy`]).
    pub fn start_deploy(&self, game: &GameInstall, then_play: bool) -> Result<(), VmmError> {
        if let Some(why) = self.blocker() {
            return Err(why);
        }
        let logs = self.data_dir.as_ref().map(|d| d.join("logs"));
        deploy::start(&self.progress, game.clone(), logs, then_play);
        Ok(())
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
        };
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

    /// Play: starts the game when it is up to date, or deploys first. Returns
    /// whether a deploy was started.
    pub fn play(&self, game: &GameInstall) -> Result<bool, VmmError> {
        let current = self.backend.deploy_fingerprint(game).ok();
        if current.is_some() && current == self.record.borrow().fingerprint {
            deploy::launch(game)?;
            Ok(false)
        } else {
            self.start_deploy(game, true)?;
            Ok(true)
        }
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
    fn enabled_order_follows_load_positions() {
        let m = |id: &str, pos: Option<usize>| vmm_core::backend::ModEntry {
            id: id.into(),
            load_position: pos,
            ..Default::default()
        };
        assert_eq!(enabled_order(&[m("b", Some(2)), m("off", None), m("a", Some(1))]), ["a", "b"]);
    }
}
