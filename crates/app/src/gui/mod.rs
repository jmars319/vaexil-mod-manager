//! The main window. Stage 1 is read-only: it finds HITMAN and its SMF and
//! shows the mods, refreshing every few seconds.

mod view;

use std::rc::Rc;
use std::time::Duration;

use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use slint::{ComponentHandle, Model, ModelRc, SharedString, VecModel};
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

    let smf = Rc::new(Smf::default());
    refresh(&ui, &smf, &rows, &notes);

    let timer = slint::Timer::default();
    {
        let weak = ui.as_weak();
        let (smf, rows, notes) = (smf.clone(), rows.clone(), notes.clone());
        timer.start(slint::TimerMode::Repeated, Duration::from_secs(5), move || {
            if let Some(ui) = weak.upgrade() {
                refresh(&ui, &smf, &rows, &notes);
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

fn refresh(ui: &AppWindow, smf: &Smf, rows: &VecModel<ModRow>, notes: &VecModel<Message>) {
    let installs = match crate::platform::game_override() {
        Some(game) => vec![game],
        None => vmm_core::game::find_installs(&crate::platform::sources()),
    };
    let v = view::build(&installs, smf);
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
            name: r.name.into(),
            detail: r.detail.into(),
            position: r.position,
            badge: r.badge.into(),
            problem: r.problem.into(),
        })
        .collect();
    sync(rows, fresh);
    sync(notes, v.notes.into_iter().map(|n| Message { text: SharedString::from(n), error: false }).collect());
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
