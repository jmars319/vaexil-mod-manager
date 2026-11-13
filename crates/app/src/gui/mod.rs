//! The main window. It finds HITMAN and its SMF, shows the mods grouped by
//! kind, refreshes every few seconds, turns mods on or off in SMF's
//! `config.json` when a switch is used, opens the edit sheet for a mod's
//! type and options, and shows the Load order view, whose changes are a draft
//! until Save. It also deploys and starts the game, keeps setups, and runs
//! compatibility passes (see `wire`).

mod order_view;
mod play;
mod sheet;
mod view;
mod wire;

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::Duration;

use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use slint::{ComponentHandle, Model, ModelRc, SharedString, VecModel};
use vmm_core::framework::{Framework, ModEntry};
use vmm_core::game::GameInstall;
use vmm_core::smf::Smf;

use crate::settings::Settings;
use crate::setups::Setups;
use crate::{AppWindow, EditState, Message, ModRow, OptCheck, OptSelect, OrderRowData, OrderState, Ui};

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
    let smf = Rc::new(match &data_dir {
        Some(dir) => Smf::with_backups(dir.join("config-backups")),
        None => Smf::default(),
    });
    let settings = match &data_dir {
        Some(dir) => Settings::load(dir),
        None => Ok(Settings::default()),
    };
    let setups = match &data_dir {
        Some(dir) => Setups::load(dir),
        None => Ok(Setups::default()),
    };
    let record = data_dir.as_deref().map(play::Record::load).unwrap_or_default();
    let state = Rc::new(State {
        smf,
        rows,
        notes,
        game: RefCell::default(),
        error: RefCell::default(),
        data_dir,
        settings: RefCell::new(settings),
        sheet: RefCell::default(),
        checks: Rc::new(VecModel::default()),
        selects: Rc::new(VecModel::default()),
        mods: RefCell::default(),
        draft: RefCell::default(),
        order_note: RefCell::default(),
        order_rows: Rc::new(VecModel::default()),
        deploy: play::Shared::default(),
        record: RefCell::new(record),
        setups: RefCell::new(setups),
        pass_candidates: RefCell::default(),
        pass_picked: RefCell::default(),
    });
    g.set_order_rows(ModelRc::from(state.order_rows.clone()));
    let es = ui.global::<EditState>();
    es.set_checks(ModelRc::from(state.checks.clone()));
    es.set_selects(ModelRc::from(state.selects.clone()));
    refresh(&ui, &state);
    wire_sheet(&ui, &state);
    wire_order(&ui, &state);
    let _deploy_timer = wire::wire_play(&ui, &state);
    wire::wire_setups(&ui, &state);
    wire::wire_pass(&ui, &state);
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
    data_dir: Option<PathBuf>,
    /// The app's own settings, or why they can't be read. Unreadable
    /// settings are never overwritten.
    settings: RefCell<Result<Settings, String>>,
    /// The open edit sheet, if any.
    sheet: RefCell<Option<sheet::EditSheet>>,
    checks: Rc<VecModel<OptCheck>>,
    selects: Rc<VecModel<OptSelect>>,
    /// The mods from the last refresh.
    mods: RefCell<Vec<ModEntry>>,
    /// The Load order view's unsaved order, if it has one.
    draft: RefCell<Option<Vec<String>>>,
    /// A line about the last load-order action, such as what Auto-sort did.
    order_note: RefCell<Option<Message>>,
    order_rows: Rc<VecModel<OrderRowData>>,
    /// The deploy in progress, shared with its worker thread.
    deploy: play::Shared,
    /// The last finished deploy.
    record: RefCell<play::Record>,
    /// Saved setups and the compatibility pass, or why they can't be read.
    setups: RefCell<Result<Setups, String>>,
    /// The ids behind the pass view's candidate rows, in row order.
    pass_candidates: RefCell<Vec<String>>,
    /// The candidates ticked for the next batch.
    pass_picked: RefCell<Vec<String>>,
}

/// The Load order view: moves and Auto-sort change a draft; Save writes it.
fn wire_order(ui: &AppWindow, state: &Rc<State>) {
    let g = ui.global::<Ui>();
    let order_now =
        |state: &State| state.draft.borrow().clone().unwrap_or_else(|| order_view::current_order(&state.mods.borrow()));
    {
        let (weak, state) = (ui.as_weak(), state.clone());
        g.on_show_order(move |show| {
            let Some(ui) = weak.upgrade() else { return };
            ui.global::<Ui>().set_view(if show { "order".into() } else { "mods".into() });
            refresh(&ui, &state);
        });
    }
    // Rows in order.slint can't reach Ui, so they call OrderState, which
    // forwards here.
    {
        let weak = ui.as_weak();
        ui.global::<OrderState>().on_move_mod(move |index, delta| {
            if let Some(ui) = weak.upgrade() {
                ui.global::<Ui>().invoke_move_mod(index, delta);
            }
        });
    }
    {
        let (weak, state) = (ui.as_weak(), state.clone());
        g.on_move_mod(move |index, delta| {
            let Some(ui) = weak.upgrade() else { return };
            let next = order_view::moved(&order_now(&state), index.max(0) as usize, delta);
            set_draft(&state, next);
            *state.order_note.borrow_mut() = None;
            refresh(&ui, &state);
        });
    }
    {
        let (weak, state) = (ui.as_weak(), state.clone());
        g.on_auto_sort(move || {
            let Some(ui) = weak.upgrade() else { return };
            let current = order_now(&state);
            let mods = state.mods.borrow().clone();
            let note = match vmm_core::order::auto_sort(&current, &mods) {
                Ok(sorted) if sorted == current => {
                    Message { text: "Auto-sort found nothing to change: every load rule is kept.".into(), error: false }
                }
                Ok(sorted) => {
                    let moved = sorted.iter().zip(&current).filter(|(a, b)| a != b).count();
                    set_draft(&state, sorted);
                    Message {
                        text: format!(
                            "Auto-sort changed the places of {moved} mods to keep their load rules. Save to keep it."
                        )
                        .into(),
                        error: false,
                    }
                }
                Err(caught) => {
                    let names: Vec<String> = caught
                        .iter()
                        .map(|id| mods.iter().find(|m| m.id == *id).map_or(id.clone(), |m| m.name.clone()))
                        .collect();
                    Message {
                        text: format!(
                            "These mods' load rules contradict each other, so Auto-sort changed nothing: {}. \
                             Order them by hand, or turn one off.",
                            names.join(", ")
                        )
                        .into(),
                        error: true,
                    }
                }
            };
            *state.order_note.borrow_mut() = Some(note);
            refresh(&ui, &state);
        });
    }
    {
        let (weak, state) = (ui.as_weak(), state.clone());
        g.on_save_order(move || {
            let Some(ui) = weak.upgrade() else { return };
            let Some(draft) = state.draft.borrow().clone() else { return };
            let game = state.game.borrow().clone();
            let result = match &game {
                _ if state.data_dir.is_none() => Err("Windows didn't say where your local app data folder is \
                     (%LOCALAPPDATA%), so there is nowhere to keep a copy of SMF's config.json. Nothing was saved."
                    .to_string()),
                Some(game) => state.smf.set_load_order(game, &draft),
                None => Err("HITMAN is no longer where it was. Nothing was saved.".to_string()),
            };
            *state.order_note.borrow_mut() = Some(match result {
                Ok(_) => {
                    *state.draft.borrow_mut() = None;
                    Message { text: "Load order saved. It reaches the game at the next deploy.".into(), error: false }
                }
                Err(e) => Message { text: format!("Couldn't save the load order. {e}").into(), error: true },
            });
            refresh(&ui, &state);
        });
    }
    {
        let (weak, state) = (ui.as_weak(), state.clone());
        g.on_discard_order(move || {
            let Some(ui) = weak.upgrade() else { return };
            *state.draft.borrow_mut() = None;
            *state.order_note.borrow_mut() = None;
            refresh(&ui, &state);
        });
    }
}

/// A draft equal to the saved order is no draft at all.
fn set_draft(state: &State, next: Vec<String>) {
    let saved = order_view::current_order(&state.mods.borrow());
    *state.draft.borrow_mut() = if next == saved { None } else { Some(next) };
}

/// The edit sheet: opened from a row, changed in place, written on Save.
fn wire_sheet(ui: &AppWindow, state: &Rc<State>) {
    let g = ui.global::<Ui>();
    let es = ui.global::<EditState>();
    {
        let (weak, state) = (ui.as_weak(), state.clone());
        g.on_edit(move |id| {
            let Some(ui) = weak.upgrade() else { return };
            let game = state.game.borrow().clone();
            let Some(game) = game else { return };
            let Some(m) = state.smf.inspect(&game).mods.into_iter().find(|m| m.id == id.as_str()) else { return };
            let chosen = state.settings.borrow().as_ref().ok().and_then(|s| s.kind_overrides.get(&m.id).copied());
            let s = sheet::EditSheet::new(&m, m.kind, chosen);
            show_sheet(&ui, &state, &s, "");
            *state.sheet.borrow_mut() = Some(s);
            ui.global::<Ui>().set_sheet("edit".into());
        });
    }
    {
        let (weak, state) = (ui.as_weak(), state.clone());
        es.on_cancel(move || {
            *state.sheet.borrow_mut() = None;
            if let Some(ui) = weak.upgrade() {
                ui.global::<Ui>().set_sheet("".into());
            }
        });
    }
    {
        let state = state.clone();
        es.on_check(move |i, on| {
            if let Some(s) = state.sheet.borrow_mut().as_mut()
                && let Some(c) = s.checks.get_mut(i as usize)
            {
                c.checked = on;
            }
        });
    }
    {
        let state = state.clone();
        es.on_select(move |i, index| {
            if let Some(s) = state.sheet.borrow_mut().as_mut()
                && let Some(sel) = s.selects.get_mut(i as usize)
                && (index as usize) < sel.values.len()
            {
                sel.index = index as usize;
            }
        });
    }
    {
        let state = state.clone();
        es.on_kind(move |index| {
            if let Some(s) = state.sheet.borrow_mut().as_mut() {
                s.kind_index = index.max(0) as usize;
            }
        });
    }
    {
        let (weak, state) = (ui.as_weak(), state.clone());
        es.on_save(move || {
            let Some(ui) = weak.upgrade() else { return };
            let Some(s) = state.sheet.borrow().clone() else { return };
            match save_sheet(&state, &s) {
                Ok(()) => {
                    *state.sheet.borrow_mut() = None;
                    ui.global::<Ui>().set_sheet("".into());
                    refresh(&ui, &state);
                }
                Err(e) => ui.global::<EditState>().set_error(e.into()),
            }
        });
    }
}

fn show_sheet(ui: &AppWindow, state: &State, s: &sheet::EditSheet, error: &str) {
    let es = ui.global::<EditState>();
    es.set_title(s.title.clone().into());
    es.set_subtitle(s.subtitle.clone().into());
    es.set_kinds(ModelRc::new(VecModel::from(
        s.kind_labels.iter().map(|l| SharedString::from(l.as_str())).collect::<Vec<_>>(),
    )));
    es.set_kind_index(s.kind_index as i32);
    let settings_error = state.settings.borrow().as_ref().err().cloned();
    es.set_kind_enabled(settings_error.is_none() && state.data_dir.is_some());
    es.set_note(settings_error.unwrap_or_default().into());
    state.checks.set_vec(
        s.checks
            .iter()
            .map(|c| OptCheck {
                label: c.label.clone().into(),
                detail: c.detail.clone().into(),
                checked: c.checked,
                enabled: c.enabled,
            })
            .collect::<Vec<_>>(),
    );
    state.selects.set_vec(
        s.selects
            .iter()
            .map(|sel| OptSelect {
                group: sel.group.clone().into(),
                labels: ModelRc::new(VecModel::from(
                    sel.labels.iter().map(|l| SharedString::from(l.as_str())).collect::<Vec<_>>(),
                )),
                index: sel.index as i32,
                enabled: sel.enabled(),
            })
            .collect::<Vec<_>>(),
    );
    es.set_error(error.into());
}

/// Writes what changed: the options to the framework, the type to the app's
/// own settings. Options go first, so a refused choice changes nothing.
fn save_sheet(state: &State, s: &sheet::EditSheet) -> Result<(), String> {
    if s.options_changed() {
        let game = state.game.borrow().clone().ok_or("HITMAN is no longer where it was. Nothing was changed.")?;
        if state.data_dir.is_none() {
            return Err("Windows didn't say where your local app data folder is (%LOCALAPPDATA%), so there is \
                        nowhere to keep a copy of SMF's config.json. Nothing was changed."
                .into());
        }
        state.smf.set_options(&game, &s.mod_id, &s.chosen())?;
    }
    if s.kind_changed() {
        let dir = state.data_dir.clone().ok_or("There is no folder to keep the type in. The type was not changed.")?;
        // Read the file again rather than trusting what was read at start: a
        // second window may have saved since, and its change must survive.
        let mut settings = Settings::load(&dir)?;
        match s.kind() {
            Some(k) => settings.kind_overrides.insert(s.mod_id.clone(), k),
            None => settings.kind_overrides.remove(&s.mod_id),
        };
        settings.save(&dir)?;
        *state.settings.borrow_mut() = Ok(settings);
    }
    Ok(())
}

fn refresh(ui: &AppWindow, state: &State) {
    let installs = match crate::platform::game_override() {
        Some(game) => vec![game],
        None => vmm_core::game::find_installs(&crate::platform::sources()),
    };
    *state.game.borrow_mut() = installs.first().cloned();
    // Reread the settings, so a type saved in another window shows here too.
    if let Some(dir) = &state.data_dir {
        *state.settings.borrow_mut() = Settings::load(dir);
    }
    let kinds = state.settings.borrow().as_ref().map(|s| s.kind_overrides.clone()).unwrap_or_default();
    let v = view::build(&installs, state.smf.as_ref(), &kinds);
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
            can_edit: r.can_edit,
            notice: r.notice.into(),
        })
        .collect();
    sync(&state.rows, fresh);

    // The Load order view. A draft whose mods no longer match the ones that
    // are on (something was turned on or off meanwhile) can't be saved.
    *state.mods.borrow_mut() = v.mods.clone();
    let saved = order_view::current_order(&v.mods);
    let stale = state.draft.borrow().as_ref().is_some_and(|d| {
        let (mut a, mut b) = (d.clone(), saved.clone());
        a.sort();
        b.sort();
        a != b
    });
    if stale {
        *state.draft.borrow_mut() = None;
        *state.order_note.borrow_mut() = Some(Message {
            text: "A mod was turned on or off while you were reordering, so the unsaved order was discarded.".into(),
            error: true,
        });
    }
    let order = state.draft.borrow().clone().unwrap_or(saved);
    let dirty = state.draft.borrow().is_some();
    let order_rows: Vec<OrderRowData> = order_view::rows(&order, &v.mods, &kinds)
        .into_iter()
        .map(|r| OrderRowData {
            id: r.id.into(),
            name: r.name.into(),
            position: r.position,
            detail: r.detail.into(),
            notice: r.notice.into(),
            can_up: r.can_up,
            can_down: r.can_down,
        })
        .collect();
    sync(&state.order_rows, order_rows);
    g.set_order_summary(order_view::summary(&order, &v.mods, dirty).into());
    g.set_order_dirty(dirty);
    let note = state.order_note.borrow().clone().unwrap_or(Message { text: "".into(), error: false });
    g.set_order_note(note.text);
    g.set_order_note_error(note.error);
    wire::refresh_extras(ui, state);
    let error = state.error.borrow().clone().map(|text| Message { text: text.into(), error: true });
    let settings_error =
        state.settings.borrow().as_ref().err().map(|text| Message { text: text.clone().into(), error: false });
    let notes = error
        .into_iter()
        .chain(settings_error)
        .chain(v.notes.into_iter().map(|n| Message { text: SharedString::from(n), error: false }));
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
