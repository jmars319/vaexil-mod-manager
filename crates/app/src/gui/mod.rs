//! The main window. It finds HITMAN and its SMF, shows the mods grouped by
//! kind, refreshes every few seconds, and turns mods on or off in SMF's
//! `config.json` when a switch is used.

mod view;

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use slint::{ComponentHandle, Model, ModelRc, SharedString, VecModel};
use vmm_core::framework::Framework;
use vmm_core::game::GameInstall;
use vmm_core::smf::Smf;

use crate::{AppWindow, Message, ModRow, Ui};

pub fn run() -> i32 {
    let ui = match AppWindow::new() {
        Ok(ui) => ui,
        Err(e) => {
            crate::show_message("Vaexil Mod Manager could not start", &e.to_string());
            return 1;
        }
    };
    let rows = Rc::new(VecModel::<ModRow>::default());
    let notes = Rc::new(VecModel::<Message>::default());
    let g = ui.global::<Ui>();
    g.set_version(env!("CARGO_PKG_VERSION").into());
    g.set_mods(ModelRc::from(rows.clone()));
    g.set_notes(ModelRc::from(notes.clone()));

    // Without its own folder the app can't keep a copy of config.json, so
    // it reads but never writes.
    let data_dir = crate::platform::data_dir();
    let can_write = data_dir.is_some();
    let smf = Rc::new(match data_dir {
        Some(dir) => Smf::with_backups(dir.join("config-backups")),
        None => Smf::default(),
    });
    let state = Rc::new(State { smf, rows, notes, game: RefCell::default(), error: RefCell::default() });
    refresh(&ui, &state);

    {
        let weak = ui.as_weak();
        let state = state.clone();
        g.on_toggle(move |id, on| {
            let Some(ui) = weak.upgrade() else { return };
            let game = state.game.borrow().clone();
            let result = match &game {
                _ if !can_write => Err("Windows didn't say where your local app data folder is (%LOCALAPPDATA%), \
                                        so there is nowhere to keep a copy of SMF's config.json. Nothing was changed."
                    .to_string()),
                Some(game) => state.smf.set_enabled(game, &id, on),
                None => Err("HITMAN is no longer where it was. Nothing was changed.".to_string()),
            };
            *state.error.borrow_mut() =
                result.err().map(|e| format!("Couldn't turn that mod {}. {e}", if on { "on" } else { "off" }));
            refresh(&ui, &state);
        });
    }

    let timer = slint::Timer::default();
    {
        let weak = ui.as_weak();
        let state = state.clone();
        timer.start(slint::TimerMode::Repeated, Duration::from_secs(5), move || {
            if let Some(ui) = weak.upgrade() {
                refresh(&ui, &state);
            }
        });
    }
    let _repaint = repaint_after_restore(&ui);
    match ui.run() {
        Ok(()) => 0,
        Err(e) => {
            crate::show_message("Vaexil Mod Manager stopped", &e.to_string());
            1
        }
    }
}

/// What the window's callbacks share.
struct State {
    smf: Rc<Smf>,
    rows: Rc<VecModel<ModRow>>,
    notes: Rc<VecModel<Message>>,
    /// The install the list shows, which a switch acts on.
    game: RefCell<Option<GameInstall>>,
    /// The last change that failed, shown until the next one succeeds.
    error: RefCell<Option<String>>,
}

fn refresh(ui: &AppWindow, state: &State) {
    let installs = match crate::platform::game_override() {
        Some(game) => vec![game],
        None => vmm_core::game::find_installs(&crate::platform::sources()),
    };
    *state.game.borrow_mut() = installs.first().cloned();
    let v = view::build(&installs, state.smf.as_ref());
    let g = ui.global::<Ui>();
    g.set_screen(match v.screen {
        view::Screen::Mods => "mods".into(),
        view::Screen::Message => "message".into(),
    });
    g.set_game_detail(v.game_detail.into());
    g.set_chip(v.chip.into());
    g.set_chip_warn(v.chip_warn);
    g.set_heading(v.heading.into());
    g.set_message(v.message.into());
    g.set_summary(v.summary.into());
    // Update models in place, so focus and scroll survive the refresh.
    let fresh: Vec<ModRow> = v
        .rows
        .into_iter()
        .map(|r| ModRow {
            heading: r.heading.into(),
            id: r.id.into(),
            name: r.name.into(),
            detail: r.detail.into(),
            position: r.position,
            badge: r.badge.into(),
            problem: r.problem.into(),
            can_toggle: r.can_toggle,
        })
        .collect();
    sync(&state.rows, fresh);
    let error = state.error.borrow().clone().map(|text| Message { text: text.into(), error: true });
    let notes =
        error.into_iter().chain(v.notes.into_iter().map(|n| Message { text: SharedString::from(n), error: false }));
    sync(&state.notes, notes.collect());
}

fn sync<T: Clone + PartialEq + 'static>(model: &VecModel<T>, fresh: Vec<T>) {
    if model.row_count() != fresh.len() {
        model.set_vec(fresh);
        return;
    }
    for (i, item) in fresh.into_iter().enumerate() {
        if model.row_data(i).as_ref() != Some(&item) {
            model.set_row_data(i, item);
        }
    }
}

/// Forces full repaints for a moment after the window is restored from
/// minimized. Slint's software renderer otherwise repaints only changed items
/// and leaves the rest of the restored window unpainted (found in Modlist
/// Launcher). Restoring passes through several sizes, so a few frames are forced.
fn repaint_after_restore(ui: &AppWindow) -> slint::Timer {
    const FORCED_FRAMES: u8 = 4;
    let weak = ui.as_weak();
    let mut was_minimized = false;
    let mut pending = 0u8;
    let timer = slint::Timer::default();
    timer.start(slint::TimerMode::Repeated, Duration::from_millis(150), move || {
        let Some(ui) = weak.upgrade() else { return };
        let minimized = window_hwnd(&ui).is_some_and(crate::platform::is_minimized);
        if was_minimized && !minimized {
            pending = FORCED_FRAMES;
        }
        was_minimized = minimized;
        if pending > 0 && !minimized {
            pending -= 1;
            ui.set_repaint_flip(!ui.get_repaint_flip());
            ui.window().request_redraw();
        }
    });
    timer
}

fn window_hwnd(ui: &AppWindow) -> Option<windows::Win32::Foundation::HWND> {
    let handle = ui.window().window_handle();
    match handle.window_handle().ok()?.as_raw() {
        RawWindowHandle::Win32(h) => Some(windows::Win32::Foundation::HWND(h.hwnd.get() as *mut _)),
        _ => None,
    }
}
