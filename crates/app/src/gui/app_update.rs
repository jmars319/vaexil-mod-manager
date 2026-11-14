//! Settings' Vaexil Mod Manager card: whether a newer version is out, and
//! updating to it (E8). The feed, download and checks are in
//! `vmm_engine::app_update`; the network runs off the window's thread.

use std::rc::Rc;

use slint::ComponentHandle;
use vmm_engine::app_update;

use super::State;
use super::settings::{in_background, in_background_to};
use crate::{AppWindow, SettingsState};

/// The version line and badge, from the last check.
pub fn show(ui: &AppWindow, state: &State) {
    let ss = ui.global::<SettingsState>();
    let s = state.engine.app_update_status();
    let checked = s.checked_at_ms.map(|t| format!(" Checked {} UTC.", vmm_engine::events::timestamp(t)));
    let (text, badge) = match (&s.latest, &checked) {
        (Some(v), _) => (
            format!("This is {}. Version {v} is available.{}", s.current, checked.clone().unwrap_or_default()),
            "available",
        ),
        (None, Some(c)) => (format!("This is {}, the newest.{c}", s.current), "current"),
        (None, None) => (format!("This is {}. Not checked yet.", s.current), "unchecked"),
    };
    ss.set_app_status(text.into());
    ss.set_app_state(badge.into());
}

fn note(ui: &AppWindow, text: String, error: bool) {
    let ss = ui.global::<SettingsState>();
    ss.set_app_note(text.into());
    ss.set_app_note_error(error);
}

pub fn wire(ui: &AppWindow, state: &Rc<State>) {
    let ss = ui.global::<SettingsState>();
    {
        let (weak, state) = (ui.as_weak(), state.clone());
        ss.on_app_check(move || {
            let Some(ui) = weak.upgrade() else { return };
            let Some(data) = state.engine.data_dir().map(|d| d.to_path_buf()) else { return };
            ui.global::<SettingsState>().set_app_busy(true);
            note(&ui, "Asking the release feed…".into(), false);
            let state = state.clone();
            in_background(
                &ui,
                move |_| app_update::check(&data).map(|_| ()),
                move |ui, r| {
                    ui.global::<SettingsState>().set_app_busy(false);
                    show(ui, &state);
                    match r {
                        Ok(()) => note(ui, String::new(), false),
                        Err(e) => note(ui, e.message, true),
                    }
                },
            );
        });
    }
    {
        let weak = ui.as_weak();
        ss.on_app_update(move || {
            if let Some(ui) = weak.upgrade() {
                note(&ui, String::new(), false);
                ui.global::<SettingsState>().set_app_confirming(true);
            }
        });
    }
    {
        let weak = ui.as_weak();
        ss.on_app_cancel(move || {
            if let Some(ui) = weak.upgrade() {
                ui.global::<SettingsState>().set_app_confirming(false);
            }
        });
    }
    {
        let (weak, state) = (ui.as_weak(), state.clone());
        ss.on_app_confirm(move || {
            let Some(ui) = weak.upgrade() else { return };
            let ss = ui.global::<SettingsState>();
            ss.set_app_confirming(false);
            if let Err(e) = state.engine.check_can_update_app() {
                note(&ui, e.message, true);
                return;
            }
            let (Some(data), Some(release)) = (
                state.engine.data_dir().map(|d| d.to_path_buf()),
                state.engine.data_dir().and_then(app_update::known_newer),
            ) else {
                note(&ui, "There is no newer version to install. Check for updates first.".into(), true);
                return;
            };
            ss.set_app_busy(true);
            note(&ui, format!("Downloading Vaexil Mod Manager {}…", release.version), false);
            let state = state.clone();
            let version = release.version.clone();
            in_background_to(
                &ui,
                move |line| app_update::download(&data, &release, &mut |l| line(l)),
                |ui, line| note(ui, line, false),
                move |ui, r| {
                    let ss = ui.global::<SettingsState>();
                    // The download took a while: a deploy or an SMF update may
                    // have started meanwhile, and closing now would cut it off.
                    let blocked = if super::settings::smf_update_running() {
                        Err(vmm_core::error::VmmError::new(
                            vmm_core::error::ErrorCode::SmfUpdating,
                            "SMF is being updated. Update Vaexil Mod Manager when it has finished; the download is kept.",
                        ))
                    } else {
                        state.engine.check_can_update_app()
                    };
                    let started = blocked.and(r).and_then(|installer| {
                        // The installer runs on its own, sets aside any file in
                        // use, and reopens the app; this window closes now.
                        std::process::Command::new(&installer)
                            .args(app_update::installer_args(true))
                            .spawn()
                            .map(|_| format!("Installing Vaexil Mod Manager {version}; it reopens when done."))
                            .map_err(|e| {
                                vmm_core::error::VmmError::new(
                                    vmm_core::error::ErrorCode::Io,
                                    format!("Couldn't start the installer: {e}"),
                                )
                            })
                    });
                    state.engine.note_app_update(&started);
                    match started {
                        Ok(m) => {
                            note(ui, m, false);
                            let _ = slint::quit_event_loop();
                        }
                        Err(e) => {
                            ss.set_app_busy(false);
                            note(ui, e.message, true);
                        }
                    }
                },
            );
        });
    }
}

/// Checks the release feed in the background when the last check is over a
/// day old, so Settings and the main window can say an update is out.
pub fn daily_check(state: &State) {
    let Some(data) = state.engine.data_dir().map(|d| d.to_path_buf()) else { return };
    let due = app_update::last_check(&data).is_none_or(|l| {
        l.current != app_update::CURRENT
            || vmm_engine::deploy::now_ms().saturating_sub(l.checked_at_ms) > vmm_engine::smf_update::CHECK_EVERY_MS
    });
    // Tests and development runs (a stand-in game) never go online by themselves.
    if due && vmm_engine::host::game_override().is_none() {
        std::thread::spawn(move || {
            let _ = app_update::check(&data);
        });
    }
}
