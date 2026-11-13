//! The window's Deploy and Play, setups, and compatibility-pass actions. Each
//! one is a call to the engine; this module only turns results into words.

use std::rc::Rc;
use std::time::Duration;

use slint::{ComponentHandle, ModelRc, SharedString, VecModel};
use vmm_engine::deploy as play;
use vmm_engine::setups::BatchStatus;

use super::{State, refresh};
use crate::{AppWindow, PassBatch, PassCandidate, PassState, PlayState, SetupsState, Ui};

/// Deploy, Play and Open log, plus the timer that follows a running deploy.
pub fn wire_play(ui: &AppWindow, state: &Rc<State>) -> slint::Timer {
    let g = ui.global::<Ui>();
    {
        let (weak, state) = (ui.as_weak(), state.clone());
        g.on_deploy(move || {
            let Some(ui) = weak.upgrade() else { return };
            let Some(game) = state.game.borrow().clone() else { return };
            note(&state, state.engine.start_deploy(&game, false).err().map(|e| e.message));
            refresh(&ui, &state);
        });
    }
    {
        let (weak, state) = (ui.as_weak(), state.clone());
        g.on_play(move || {
            let Some(ui) = weak.upgrade() else { return };
            let Some(game) = state.game.borrow().clone() else { return };
            // The status line takes its own fingerprint next time.
            *state.fingerprint.borrow_mut() = None;
            if state.engine.up_to_date(&game) == Some(true) {
                note(&state, state.engine.launch_as_is(&game).err().map(|e| e.message));
            } else {
                // Changes since the last deploy: deploy first, or play as is.
                ui.global::<PlayState>().set_reason(crate::steam_launch::reason(&state.engine).into());
                ui.global::<Ui>().set_sheet("play".into());
            }
            refresh(&ui, &state);
        });
    }
    {
        let ps = ui.global::<PlayState>();
        let close = |ui: &AppWindow| ui.global::<Ui>().set_sheet("".into());
        {
            let (weak, state) = (ui.as_weak(), state.clone());
            ps.on_deploy_then_play(move || {
                let Some(ui) = weak.upgrade() else { return };
                close(&ui);
                let Some(game) = state.game.borrow().clone() else { return };
                note(&state, state.engine.start_deploy(&game, true).err().map(|e| e.message));
                refresh(&ui, &state);
            });
        }
        {
            let (weak, state) = (ui.as_weak(), state.clone());
            ps.on_play_now(move || {
                let Some(ui) = weak.upgrade() else { return };
                close(&ui);
                let Some(game) = state.game.borrow().clone() else { return };
                note(&state, state.engine.launch_as_is(&game).err().map(|e| e.message));
                refresh(&ui, &state);
            });
        }
        {
            let weak = ui.as_weak();
            ps.on_cancel(move || {
                if let Some(ui) = weak.upgrade() {
                    close(&ui);
                }
            });
        }
    }
    {
        let (weak, state) = (ui.as_weak(), state.clone());
        g.on_open_log(move || {
            let Some(ui) = weak.upgrade() else { return };
            if let Some(log) = state.engine.record().log {
                note(&state, vmm_engine::host::open(&log).err().map(|e| e.message));
                refresh(&ui, &state);
            }
        });
    }
    // While a deploy runs, follow it closely; finish up when it ends.
    let timer = slint::Timer::default();
    let (weak, state) = (ui.as_weak(), state.clone());
    timer.start(slint::TimerMode::Repeated, Duration::from_millis(300), move || {
        let Some(ui) = weak.upgrade() else { return };
        let (running, _) = state.engine.deploy_progress();
        let game = state.game.borrow().clone();
        match state.engine.finish_deploy(game.as_ref()) {
            Some(problem) => {
                // Look again after a deploy, rather than trust a cached one.
                *state.fingerprint.borrow_mut() = None;
                note(&state, problem.map(|e| e.message));
            }
            None if !running => return,
            None => {}
        }
        refresh(&ui, &state);
    });
    timer
}

fn note(state: &State, problem: Option<String>) {
    *state.error.borrow_mut() = problem;
}

/// The Setups sheet: save the mods that are on as a setup, switch to one, or
/// delete one.
pub fn wire_setups(ui: &AppWindow, state: &Rc<State>) {
    let g = ui.global::<Ui>();
    let ss = ui.global::<SetupsState>();
    {
        let (weak, state) = (ui.as_weak(), state.clone());
        g.on_show_setups(move || {
            let Some(ui) = weak.upgrade() else { return };
            let ss = ui.global::<SetupsState>();
            ss.set_error("".into());
            ss.set_name("".into());
            ui.global::<Ui>().set_sheet("setups".into());
            refresh(&ui, &state);
        });
    }
    {
        let weak = ui.as_weak();
        ss.on_cancel(move || {
            if let Some(ui) = weak.upgrade() {
                ui.global::<Ui>().set_sheet("".into());
            }
        });
    }
    {
        let (weak, state) = (ui.as_weak(), state.clone());
        ss.on_save_as(move |name| {
            let Some(ui) = weak.upgrade() else { return };
            let result = match state.game.borrow().clone() {
                Some(game) => state.engine.save_setup(&game, &name).map_err(|e| e.message),
                None => Err("HITMAN is no longer where it was.".to_string()),
            };
            match result {
                Ok(()) => ui.global::<SetupsState>().set_name("".into()),
                Err(e) => ui.global::<SetupsState>().set_error(e.into()),
            }
            refresh(&ui, &state);
        });
    }
    {
        let (weak, state) = (ui.as_weak(), state.clone());
        ss.on_use_setup(move |i| {
            let Some(ui) = weak.upgrade() else { return };
            let result = match state.game.borrow().clone() {
                Some(game) => state.engine.use_setup(&game, i.max(0) as usize).map_err(|e| e.message),
                None => Err("HITMAN is no longer where it was. Nothing was changed.".to_string()),
            };
            let msg = match result {
                Ok((name, skipped)) if skipped.is_empty() => format!("Switched to {name}."),
                Ok((name, skipped)) => {
                    format!("Switched to {name}. Not installed now, so left out: {}.", skipped.join(", "))
                }
                Err(e) => format!("Couldn't switch. {e}"),
            };
            ui.global::<SetupsState>().set_error(msg.into());
            refresh(&ui, &state);
        });
    }
    {
        let (weak, state) = (ui.as_weak(), state.clone());
        ss.on_delete_setup(move |i| {
            let Some(ui) = weak.upgrade() else { return };
            if let Err(e) = state.engine.delete_setup(i.max(0) as usize) {
                ui.global::<SetupsState>().set_error(e.message.into());
            }
            refresh(&ui, &state);
        });
    }
}

/// The compatibility pass view.
pub fn wire_pass(ui: &AppWindow, state: &Rc<State>) {
    let g = ui.global::<Ui>();
    let ps = ui.global::<PassState>();
    {
        let (weak, state) = (ui.as_weak(), state.clone());
        g.on_show_pass(move |show| {
            let Some(ui) = weak.upgrade() else { return };
            ui.global::<Ui>().set_view(if show { "pass".into() } else { "mods".into() });
            refresh(&ui, &state);
        });
    }
    // Runs a pass step through the engine. Errors show in the pass view.
    fn act(
        ui: &AppWindow,
        state: &Rc<State>,
        step: impl FnOnce(&vmm_engine::Engine, &vmm_core::game::GameInstall) -> vmm_core::error::Result<()>,
    ) {
        let result = match state.game.borrow().clone() {
            Some(game) => step(&state.engine, &game).map_err(|e| e.message),
            None => Err("HITMAN is no longer where it was. Nothing was changed.".to_string()),
        };
        ui.global::<PassState>().set_error(result.err().unwrap_or_default().into());
        refresh(ui, state);
    }
    {
        let (weak, state) = (ui.as_weak(), state.clone());
        ps.on_start(move |from_current| {
            let Some(ui) = weak.upgrade() else { return };
            act(&ui, &state, |engine, game| engine.start_pass(game, from_current));
        });
    }
    {
        let state = state.clone();
        ps.on_candidate(move |i, on| {
            if let Some(id) = state.pass_candidates.borrow().get(i as usize).cloned() {
                let mut picked = state.pass_picked.borrow_mut();
                picked.retain(|p| *p != id);
                if on {
                    picked.push(id);
                }
            }
        });
    }
    {
        let (weak, state) = (ui.as_weak(), state.clone());
        ps.on_begin(move |name| {
            let Some(ui) = weak.upgrade() else { return };
            let picked = state.pass_picked.borrow().clone();
            act(&ui, &state, |engine, game| engine.begin_batch(game, &name, picked));
            if ui.global::<PassState>().get_error().is_empty() {
                state.pass_picked.borrow_mut().clear();
                ui.global::<PassState>().set_batch_name("".into());
            }
        });
    }
    {
        let (weak, state) = (ui.as_weak(), state.clone());
        ps.on_mark(move |good, notes| {
            let Some(ui) = weak.upgrade() else { return };
            act(&ui, &state, |engine, game| engine.mark_batch(game, good, &notes));
        });
    }
    {
        let (weak, state) = (ui.as_weak(), state.clone());
        ps.on_end(move || {
            let Some(ui) = weak.upgrade() else { return };
            // Ending keeps the game as it is now; nothing in SMF changes.
            let result = state.engine.end_pass().map_err(|e| e.message);
            ui.global::<PassState>().set_error(result.err().unwrap_or_default().into());
            refresh(&ui, &state);
        });
    }
}

/// Fills the deploy line, the setup label, the Setups sheet and the pass view.
pub fn refresh_extras(ui: &AppWindow, state: &State) {
    let g = ui.global::<Ui>();
    let game = state.game.borrow().clone();
    // A full fingerprint walks every mod's files, so the status line reuses
    // one while a cheap check (config.json and the top of Mods) is unchanged,
    // for up to 30 seconds. Play and Deploy always take a fresh one.
    let current = {
        let quick = game.as_ref().map(|g| state.engine.quick_key(g)).unwrap_or_default();
        let mut cached = state.fingerprint.borrow_mut();
        let fresh = cached.as_ref().is_some_and(|(at, key, _)| *key == quick && at.elapsed() < Duration::from_secs(30));
        if !fresh {
            *cached = game
                .as_ref()
                .and_then(|g| state.engine.fingerprint(g).ok())
                .map(|f| (std::time::Instant::now(), quick.clone(), f));
        }
        cached.as_ref().map(|(_, _, f)| f.clone())
    };
    let (running, last) = state.engine.deploy_progress();
    let blocker = if running { None } else { state.engine.blocker() };
    let s = play::status(
        running,
        &last,
        &state.engine.record(),
        current.as_deref(),
        blocker.as_ref().map(|e| e.message.as_str()),
    );
    g.set_deploy_status(s.text.into());
    g.set_deploy_tone(s.tone.into());
    g.set_can_deploy(s.can_deploy && game.is_some());
    g.set_can_play(s.can_play && game.is_some());
    g.set_has_log(s.has_log);

    // Setups.
    let ss = ui.global::<SetupsState>();
    let now = game.as_ref().and_then(|g| state.engine.current_setup(g).ok());
    let setups = state.engine.setups();
    match &setups {
        Ok(setups) => {
            let names: Vec<SharedString> = setups
                .saved
                .iter()
                .map(|s| {
                    let mods = match s.setup.order.len() {
                        1 => "1 mod".to_string(),
                        n => format!("{n} mods"),
                    };
                    let here = now.as_ref().is_some_and(|n| n.order == s.setup.order);
                    format!("{} · {mods}{}", s.name, if here { " · in use" } else { "" }).into()
                })
                .collect();
            ss.set_setups(ModelRc::new(VecModel::from(names)));
            let label = match now.as_ref().and_then(|n| setups.matching(n)) {
                Some(s) => format!("Setup: {}", s.name),
                None => "Setup: not saved".into(),
            };
            g.set_setup_label(label.into());
            ss.set_unreadable("".into());
        }
        Err(e) => {
            ss.set_setups(ModelRc::new(VecModel::from(Vec::<SharedString>::new())));
            ss.set_unreadable(e.message.clone().into());
            g.set_setup_label("Setups".into());
        }
    }

    // The pass view.
    let ps = ui.global::<PassState>();
    let mods = state.mods.borrow();
    let name_of = |id: &str| mods.iter().find(|m| m.id == id).map_or(id.to_string(), |m| m.name.clone());
    let pass = setups.as_ref().ok().and_then(|s| s.pass.clone());
    ps.set_active(pass.is_some());
    let Some(pass) = pass else {
        ps.set_batches(ModelRc::new(VecModel::from(Vec::<PassBatch>::new())));
        ps.set_candidates(ModelRc::new(VecModel::from(Vec::<PassCandidate>::new())));
        return;
    };
    let baseline: Vec<String> = pass.baseline.iter().map(|id| name_of(id)).collect();
    ps.set_baseline(
        if baseline.is_empty() {
            "No mods yet: the game runs without managed mods.".to_string()
        } else {
            format!("{} mods: {}", baseline.len(), baseline.join(", "))
        }
        .into(),
    );
    let batches: Vec<PassBatch> = pass
        .batches
        .iter()
        .rev()
        .map(|b| PassBatch {
            name: b.name.clone().into(),
            detail: {
                let names: Vec<String> = b.mods.iter().map(|id| name_of(id)).collect();
                let mut d = names.join(", ");
                if !b.notes.is_empty() {
                    d.push_str(&format!(". Notes: {}", b.notes));
                }
                d.into()
            },
            status: b.status.label().into(),
        })
        .collect();
    ps.set_batches(ModelRc::new(VecModel::from(batches)));
    match pass.active() {
        Some(i) => {
            let b = &pass.batches[i];
            ps.set_testing(true);
            ps.set_testing_name(b.name.clone().into());
            let names: Vec<String> = b.mods.iter().map(|id| name_of(id)).collect();
            ps.set_testing_mods(names.join(", ").into());
        }
        None => ps.set_testing(false),
    }
    let candidates = pass.candidates(&mods);
    let picked = state.pass_picked.borrow().clone();
    *state.pass_candidates.borrow_mut() = candidates.iter().map(|m| m.id.clone()).collect();
    let rows: Vec<PassCandidate> = candidates
        .iter()
        .map(|m| PassCandidate {
            name: m.name.clone().into(),
            detail: {
                let tried: Vec<&str> = pass
                    .batches
                    .iter()
                    .filter(|b| b.status == BatchStatus::Bad && b.mods.contains(&m.id))
                    .map(|b| b.name.as_str())
                    .collect();
                if tried.is_empty() { String::new() } else { format!("Was in a bad batch: {}", tried.join(", ")) }
            }
            .into(),
            checked: picked.contains(&m.id),
        })
        .collect();
    ps.set_candidates(ModelRc::new(VecModel::from(rows)));
}
