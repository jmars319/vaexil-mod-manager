//! Steam's Play button (ADR 0003, E6): HITMAN's Steam launch options become
//! `"…\vaexil-mod-manager.exe" --steam-play -- %command%`, so pressing Play in
//! Steam starts the app, which deploys if the user wants and then runs the
//! game's own command itself (never back through Steam, so it can't loop).
//!
//! Ported from Modlist Launcher's `steam_hooks.rs`. Every write of Steam's
//! `localconfig.vdf` needs Steam closed (Steam rewrites the file on exit),
//! saves a backup in the app's data folder first, changes only the one value
//! surgically, replaces the file through a temporary file and a rename, and
//! reads it back, putting the original back if it doesn't match. Undoing it
//! restores the previous value only if the value is still the app's.

use std::path::{Path, PathBuf};

use vmm_core::error::{ErrorCode, VmmError};
use vmm_core::game::STEAM_APP_ID;
use vmm_core::schema::LogLevel;
use vmm_core::steam::{self, Account, SteamRoot};

use crate::{Engine, deploy, host, settings::Settings};

/// How many Steam settings backups are kept.
pub const BACKUPS_KEPT: usize = 20;

/// What the app changed in Steam, kept in its `settings.json` so it can be
/// undone exactly.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SteamHook {
    pub account_id: u32,
    /// HITMAN's launch options before the app changed them ("" for none).
    pub previous: String,
    /// What the app set them to.
    pub applied: String,
}

/// Steam's Play button as Settings shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SteamPlayView {
    /// The Steam account it applies to (the most recent one on this PC).
    pub account: Option<String>,
    /// HITMAN's launch options in that account now.
    pub current: String,
    /// What connecting would set them to.
    pub proposed: String,
    /// Whether Steam's Play button starts the app now.
    pub connected: bool,
    pub steam_running: bool,
    /// Why it can't be changed, if it can't.
    pub problem: Option<String>,
}

fn unavailable(message: impl Into<String>) -> VmmError {
    VmmError::new(ErrorCode::SteamUnavailable, message)
}

fn steam_running_error() -> VmmError {
    VmmError::new(
        ErrorCode::SteamRunning,
        "Steam is running, and it rewrites its settings when it exits. Exit Steam completely (Steam menu > Exit), \
         then try again. Nothing was changed.",
    )
}

/// Saves a copy of an account's Steam settings before they are changed,
/// keeping the newest [`BACKUPS_KEPT`].
fn backup(data: &Path, account_id: u32, text: &str) -> Result<PathBuf, VmmError> {
    let dir = data.join("steam-backups");
    let fail = |e: std::io::Error| {
        VmmError::new(
            ErrorCode::BackupUnavailable,
            format!("Couldn't back up Steam's settings, so nothing was changed: {e}"),
        )
    };
    std::fs::create_dir_all(&dir).map_err(fail)?;
    let prefix = format!("localconfig-{account_id}-");
    let mut old: Vec<PathBuf> = std::fs::read_dir(&dir)
        .map_err(fail)?
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.starts_with(&prefix)))
        .collect();
    old.sort();
    for p in old.iter().take(old.len().saturating_sub(BACKUPS_KEPT - 1)) {
        let _ = std::fs::remove_file(p);
    }
    let path = dir.join(format!("{prefix}{:015}.vdf", deploy::now_ms()));
    std::fs::write(&path, text).map_err(fail)?;
    Ok(path)
}

/// Replaces Steam's settings file through a temporary file and a rename.
fn write(path: &Path, text: &str) -> std::io::Result<()> {
    let tmp = path.with_extension("vdf.vaexil-mod-manager-tmp");
    std::fs::write(&tmp, text)?;
    std::fs::rename(&tmp, path).inspect_err(|_| {
        let _ = std::fs::remove_file(&tmp);
    })
}

/// Writes `updated` and checks HITMAN's launch options read back as `want`;
/// otherwise puts `original` back.
fn write_checked(path: &Path, original: &str, updated: &str, want: &str) -> Result<(), VmmError> {
    write(path, updated)
        .map_err(|e| VmmError::new(ErrorCode::SaveFailed, format!("Couldn't write Steam's settings: {e}")))?;
    let back = std::fs::read_to_string(path).ok().and_then(|t| steam::get_launch_options(&t, STEAM_APP_ID).ok());
    if back.as_deref() != Some(want) {
        let _ = write(path, original);
        return Err(VmmError::new(
            ErrorCode::SaveFailed,
            "Steam's settings didn't read back correctly, so the original file was put back.",
        ));
    }
    Ok(())
}

impl Engine {
    /// The Steam installation Steam's Play button is set in.
    /// `VAEXIL_MOD_MANAGER_STEAM` replaces it for tests, so they never touch
    /// the real Steam.
    pub fn steam_root(&self) -> Option<SteamRoot> {
        if let Some(dir) = std::env::var_os("VAEXIL_MOD_MANAGER_STEAM").filter(|d| !d.is_empty()) {
            return Some(SteamRoot::new(PathBuf::from(dir)));
        }
        host::sources().steam.map(SteamRoot::new)
    }

    fn steam_hook(&self) -> Option<SteamHook> {
        self.data_dir().and_then(|d| Settings::load(d).ok()).and_then(|s| s.steam_play)
    }

    fn account(&self, root: &SteamRoot) -> Result<Account, VmmError> {
        let accounts = root.accounts();
        // The account the app connected before, else the one that last signed in.
        let wanted = self.steam_hook().map(|h| h.account_id);
        wanted
            .and_then(|id| accounts.iter().find(|a| a.account_id == id).cloned())
            .or_else(|| accounts.into_iter().next())
            .ok_or_else(|| {
                unavailable("No Steam account has signed in on this PC, so there are no settings to change.")
            })
    }

    /// Steam's Play button, for Settings. `app_exe` is the app's own exe.
    pub fn steam_play_view(&self, app_exe: &Path) -> SteamPlayView {
        let proposed = steam::wrapper_launch_options(app_exe);
        let steam_running = !host::running(&["steam.exe"]).is_empty();
        let mut view = SteamPlayView {
            account: None,
            current: String::new(),
            proposed,
            connected: false,
            steam_running,
            problem: None,
        };
        let Some(root) = self.steam_root() else {
            view.problem = Some("Steam wasn't found on this PC.".into());
            return view;
        };
        match self.account(&root) {
            Ok(account) => {
                view.account = Some(account.label());
                match std::fs::read_to_string(&account.localconfig)
                    .map_err(|e| e.to_string())
                    .and_then(|t| steam::get_launch_options(&t, STEAM_APP_ID).map_err(|e| e.to_string()))
                {
                    Ok(current) => {
                        view.connected = steam::is_ours(&current);
                        view.current = current;
                    }
                    Err(e) => view.problem = Some(format!("Steam's settings can't be read: {e}")),
                }
            }
            Err(e) => view.problem = Some(e.message),
        }
        view
    }

    /// Makes Steam's Play button start HITMAN through the app.
    pub fn connect_steam_play(&self, app_exe: &Path) -> Result<String, VmmError> {
        let root = self.steam_root().ok_or_else(|| unavailable("Steam wasn't found on this PC."))?;
        let steam_running = !host::running(&["steam.exe"]).is_empty();
        let r = self.connect_steam_play_in(&root, app_exe, steam_running);
        self.note_steam("steam.play_connected", &r);
        r
    }

    /// Puts HITMAN's launch options back to what they were before.
    pub fn disconnect_steam_play(&self) -> Result<String, VmmError> {
        let root = self.steam_root().ok_or_else(|| unavailable("Steam wasn't found on this PC."))?;
        let steam_running = !host::running(&["steam.exe"]).is_empty();
        let r = self.disconnect_steam_play_in(&root, steam_running);
        self.note_steam("steam.play_restored", &r);
        r
    }

    fn note_steam(&self, event: &str, r: &Result<String, VmmError>) {
        match r {
            Ok(m) => self.log(LogLevel::Info, event, None, None, m.clone(), None),
            Err(e) => self.log(LogLevel::Error, event, None, None, e.message.clone(), Some(e)),
        }
    }

    pub(crate) fn connect_steam_play_in(
        &self,
        root: &SteamRoot,
        app_exe: &Path,
        steam_running: bool,
    ) -> Result<String, VmmError> {
        if steam_running {
            return Err(steam_running_error());
        }
        let data = self.data_dir().ok_or_else(|| {
            VmmError::new(ErrorCode::BackupUnavailable, "There is no folder to keep a backup of Steam's settings in.")
        })?;
        let mut settings = Settings::load(data)?;
        let account = self.account(root)?;
        let text = std::fs::read_to_string(&account.localconfig)
            .map_err(|e| unavailable(format!("Couldn't read Steam's settings: {e}")))?;
        let current = steam::get_launch_options(&text, STEAM_APP_ID).map_err(|e| unavailable(e.to_string()))?;
        let proposed = steam::wrapper_launch_options(app_exe);
        if current != proposed {
            let updated =
                steam::set_launch_options(&text, STEAM_APP_ID, &proposed).map_err(|e| unavailable(e.to_string()))?;
            backup(data, account.account_id, &text)?;
            write_checked(&account.localconfig, &text, &updated, &proposed)?;
        }
        // The value from before the app, kept across reconnects (say, after the
        // app moved): never record the app's own value as the one to restore.
        let previous = match settings.steam_play.take() {
            Some(h) if h.account_id == account.account_id => h.previous,
            _ if steam::is_ours(&current) => String::new(),
            _ => current,
        };
        settings.steam_play = Some(SteamHook { account_id: account.account_id, previous, applied: proposed });
        settings.save(data)?;
        Ok(format!(
            "Steam's Play button for HITMAN now starts Vaexil Mod Manager ({}). A copy of Steam's settings was kept.",
            account.label()
        ))
    }

    pub(crate) fn disconnect_steam_play_in(&self, root: &SteamRoot, steam_running: bool) -> Result<String, VmmError> {
        if steam_running {
            return Err(steam_running_error());
        }
        let data = self.data_dir().ok_or_else(|| {
            VmmError::new(ErrorCode::BackupUnavailable, "There is no folder to keep a backup of Steam's settings in.")
        })?;
        let mut settings = Settings::load(data)?;
        let hook = settings.steam_play.clone().ok_or_else(|| {
            unavailable("Steam's Play button wasn't changed by Vaexil Mod Manager, so there is nothing to undo.")
        })?;
        let account = root
            .accounts()
            .into_iter()
            .find(|a| a.account_id == hook.account_id)
            .ok_or_else(|| unavailable(format!("Steam account {} isn't on this PC any more.", hook.account_id)))?;
        let text = std::fs::read_to_string(&account.localconfig)
            .map_err(|e| unavailable(format!("Couldn't read Steam's settings: {e}")))?;
        let current = steam::get_launch_options(&text, STEAM_APP_ID).map_err(|e| unavailable(e.to_string()))?;
        let message = if steam::is_ours(&current) {
            // Ours (this exe or an older path of it): put the previous value back.
            let updated = steam::set_launch_options(&text, STEAM_APP_ID, &hook.previous)
                .map_err(|e| unavailable(e.to_string()))?;
            backup(data, account.account_id, &text)?;
            write_checked(&account.localconfig, &text, &updated, &hook.previous)?;
            "HITMAN's Steam launch options are back to what they were.".to_string()
        } else {
            // Someone set their own options since: theirs win.
            "HITMAN's Steam launch options were changed outside Vaexil Mod Manager, so they were left as they are."
                .to_string()
        };
        settings.steam_play = None;
        settings.save(data)?;
        Ok(message)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const STEAMID64_BASE: u64 = 76_561_197_960_265_728;

    /// A fake Steam with one account whose HITMAN launch options are `options`.
    fn fake_steam(root: &Path, options: &str) -> PathBuf {
        let id = 96_737_131u32;
        std::fs::create_dir_all(root.join("config")).unwrap();
        std::fs::write(
            root.join("config/loginusers.vdf"),
            format!(
                "\"users\"\n{{\n\t\"{}\"\n\t{{\n\t\t\"PersonaName\"\t\t\"Owner\"\n\t\t\"MostRecent\"\t\t\"1\"\n\t}}\n}}\n",
                STEAMID64_BASE + u64::from(id)
            ),
        )
        .unwrap();
        let cfg = root.join(format!("userdata/{id}/config"));
        std::fs::create_dir_all(&cfg).unwrap();
        let path = cfg.join("localconfig.vdf");
        std::fs::write(
            &path,
            format!(
                "\"UserLocalConfigStore\"\n{{\n\t\"Software\"\n\t{{\n\t\t\"Valve\"\n\t\t{{\n\t\t\t\"Steam\"\n\t\t\t{{\n\t\t\t\t\"apps\"\n\t\t\t\t{{\n\t\t\t\t\t\"1659040\"\n\t\t\t\t\t{{\n\t\t\t\t\t\t\"LaunchOptions\"\t\t\"{options}\"\n\t\t\t\t\t\t\"Playtime\"\t\t\"500\"\n\t\t\t\t\t}}\n\t\t\t\t}}\n\t\t\t}}\n\t\t}}\n\t}}\n}}\n"
            ),
        )
        .unwrap();
        path
    }

    #[test]
    fn connecting_and_undoing_puts_steam_back_byte_for_byte() {
        let tmp = tempfile::tempdir().unwrap();
        let localconfig = fake_steam(&tmp.path().join("steam"), "-skip_launcher");
        let before = std::fs::read(&localconfig).unwrap();
        let root = SteamRoot::new(tmp.path().join("steam"));
        let engine = Engine::with_data_dir(Some(tmp.path().join("data")));
        let exe = Path::new(r"C:\Apps\VMM\vaexil-mod-manager.exe");

        // Refused while Steam runs; nothing written.
        assert_eq!(engine.connect_steam_play_in(&root, exe, true).unwrap_err().code, ErrorCode::SteamRunning);
        assert_eq!(std::fs::read(&localconfig).unwrap(), before);

        engine.connect_steam_play_in(&root, exe, false).unwrap();
        let text = std::fs::read_to_string(&localconfig).unwrap();
        assert_eq!(steam::get_launch_options(&text, STEAM_APP_ID).unwrap(), steam::wrapper_launch_options(exe));
        assert_eq!(std::fs::read_dir(tmp.path().join("data/steam-backups")).unwrap().count(), 1);
        // Connecting again (the app moved) keeps the original to restore.
        engine.connect_steam_play_in(&root, Path::new(r"D:\VMM\vaexil-mod-manager.exe"), false).unwrap();
        assert_eq!(engine.steam_hook().unwrap().previous, "-skip_launcher");

        engine.disconnect_steam_play_in(&root, false).unwrap();
        assert_eq!(std::fs::read(&localconfig).unwrap(), before);
        assert!(engine.steam_hook().is_none());
    }

    #[test]
    fn launch_options_changed_by_hand_are_left_alone() {
        let tmp = tempfile::tempdir().unwrap();
        let localconfig = fake_steam(&tmp.path().join("steam"), "");
        let root = SteamRoot::new(tmp.path().join("steam"));
        let engine = Engine::with_data_dir(Some(tmp.path().join("data")));
        engine.connect_steam_play_in(&root, Path::new(r"C:\VMM\vaexil-mod-manager.exe"), false).unwrap();
        // The user sets their own options in Steam afterwards.
        let text = std::fs::read_to_string(&localconfig).unwrap();
        std::fs::write(&localconfig, steam::set_launch_options(&text, STEAM_APP_ID, "-windowed").unwrap()).unwrap();
        let msg = engine.disconnect_steam_play_in(&root, false).unwrap();
        assert!(msg.contains("left as they are"), "{msg}");
        let text = std::fs::read_to_string(&localconfig).unwrap();
        assert_eq!(steam::get_launch_options(&text, STEAM_APP_ID).unwrap(), "-windowed");
    }
}
