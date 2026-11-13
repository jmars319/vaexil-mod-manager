//! The window's Deploy and Play, setups, and compatibility-pass actions.
//! Every change to SMF goes through the framework, with its usual backup.

use std::rc::Rc;
use std::time::Duration;

use slint::{ComponentHandle, ModelRc, SharedString, VecModel};
use vmm_core::framework::{Framework, Setup};

use super::{State, play, refresh};
use crate::setups::{Pass, Setups};
use crate::{AppWindow, PassBatch, PassCandidate, PassState, SetupsState, Ui};

/// Deploy, Play and Open log, plus the timer that follows a running deploy.
pub fn wire_play(ui: &AppWindow, state: &Rc<State>) -> slint::Timer {
    let g = ui.global::<Ui>();
    {
        let (weak, state) = (ui.as_weak(), state.clone());
        g.on_deploy(move || {
            if let Some(ui) = weak.upgrade() {
                start_deploy(&ui, &state, false);
            }
        });
    }
    {
        let (weak, state) = (ui.as_weak(), state.clone());
        g.on_play(move || {
            let Some(ui) = weak.upgrade() else { return };
            let Some(game) = state.game.borrow().clone() else { return };
            let current = state.smf.deploy_fingerprint(&game).ok();
            if current.is_some() && current == state.record.borrow().fingerprint {
                note(&state, play::launch(&game).err());
                refresh(&ui, &state);
            } else {
                start_deploy(&ui, &state, true);
            }
        });
    }
    {
        let (weak, state) = (ui.as_weak(), state.clone());
        g.on_open_log(move || {
            let Some(ui) = weak.upgrade() else { return };
            let log = state.record.borrow().log.clone();
            if let Some(log) = log {
                note(&state, crate::platform::open(&log).err());
                refresh(&ui, &state);
            }
        });
    }
    // While a deploy runs, follow it closely; finish up when it ends.
    let timer = slint::Timer::default();
    let (weak, state) = (ui.as_weak(), state.clone());
    timer.start(slint::TimerMode::Repeated, Duration::from_millis(300), move || {
        let Some(ui) = weak.upgrade() else { return };
        let finished = {
            let mut p = state.deploy.lock().expect("deploy progress lock");
            if !p.running && p.finished.is_none() {
                return;
            }
            p.finished.take()
        };
        if let Some(f) = finished {
            let success = matches!(f.result, Ok(ref o) if o.success);
            let record = play::Record {
                fingerprint: if success { f.fingerprint.clone() } else { None },
                success: Some(success),
                exit_code: f.result.as_ref().ok().and_then(|o| o.exit_code),
                log: f.log.map(|p| p.display().to_string()),
                at_ms: Some(play::now_ms()),
            };
            if let Some(dir) = &state.data_dir {
                let _ = record.save(dir);
            }
            *state.record.borrow_mut() = record;
            let problem = match &f.result {
                Err(e) => Some(format!("Couldn't run SMF's Deploy.exe. {e}")),
                Ok(_) if !success => None,
                Ok(_) if f.then_play => state.game.borrow().as_ref().and_then(|g| play::launch(g).err()),
                Ok(_) => None,
            };
            note(&state, problem);
        }
        refresh(&ui, &state);
    });
    timer
}

fn note(state: &State, problem: Option<String>) {
    *state.error.borrow_mut() = problem;
}

fn start_deploy(ui: &AppWindow, state: &Rc<State>, then_play: bool) {
    let Some(game) = state.game.borrow().clone() else { return };
    if let Some(why) = play::blocked(&crate::platform::running(&play::BLOCKERS)) {
        note(state, Some(why));
        refresh(ui, state);
        return;
    }
    let logs = state.data_dir.as_ref().map(|d| d.join("logs"));
    play::start(&state.deploy, game, logs, then_play);
    refresh(ui, state);
}

/// Applies `order` as the mods that are on, keeping each mod's options.
fn apply_order(state: &State, order: Vec<String>) -> Result<Vec<String>, String> {
    if state.data_dir.is_none() {
        return Err("Windows didn't say where your local app data folder is (%LOCALAPPDATA%), so there is nowhere \
                    to keep a copy of SMF's config.json. Nothing was changed."
            .into());
    }
    let game = state.game.borrow().clone().ok_or("HITMAN is no longer where it was. Nothing was changed.")?;
    state.smf.apply_setup(&game, &Setup { order, options: Default::default() })
}

fn save_setups(state: &State, setups: &Setups) -> Result<(), String> {
    let dir = state.data_dir.as_ref().ok_or("There is no folder to keep setups in.")?;
    setups.save(dir)?;
    *state.setups.borrow_mut() = Ok(setups.clone());
    Ok(())
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
            let result = (|| {
                let game = state.game.borrow().clone().ok_or("HITMAN is no longer where it was.")?;
                let mut setups = state.setups.borrow().clone()?;
                setups.put(&name, state.smf.current_setup(&game)?)?;
                save_setups(&state, &setups)
            })();
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
            let result = (|| {
                let setups = state.setups.borrow().clone()?;
                let s = setups.saved.get(i as usize).ok_or("That setup is gone.")?;
                let game =
                    state.game.borrow().clone().ok_or("HITMAN is no longer where it was. Nothing was changed.")?;
                if state.data_dir.is_none() {
                    return Err(
                        "There is nowhere to keep a copy of SMF's config.json. Nothing was changed.".to_string()
                    );
                }
                let skipped = state.smf.apply_setup(&game, &s.setup)?;
                Ok((s.name.clone(), skipped))
            })();
            match result {
                Ok((name, skipped)) => {
                    let msg = if skipped.is_empty() {
                        format!("Switched to {name}.")
                    } else {
                        format!("Switched to {name}. Not installed now, so left out: {}.", skipped.join(", "))
                    };
                    ui.global::<SetupsState>().set_error(msg.into());
                }
                Err(e) => ui.global::<SetupsState>().set_error(format!("Couldn't switch. {e}").into()),
            }
            refresh(&ui, &state);
        });
    }
    {
        let (weak, state) = (ui.as_weak(), state.clone());
        ss.on_delete_setup(move |i| {
            let Some(ui) = weak.upgrade() else { return };
            let result = (|| {
                let mut setups = state.setups.borrow().clone()?;
                if (i as usize) < setups.saved.len() {
                    setups.saved.remove(i as usize);
                }
                save_setups(&state, &setups)
            })();
            if let Err(e) = result {
                ui.global::<SetupsState>().set_error(e.into());
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
    // Runs `step` on the pass, saves it, and puts the game in the pass's
    // target order. Errors show in the pass view.
    fn act(ui: &AppWindow, state: &Rc<State>, step: impl FnOnce(&State, &mut Option<Pass>) -> Result<(), String>) {
        let result = (|| {
            let mut setups = state.setups.borrow().clone()?;
            step(state, &mut setups.pass)?;
            if let Some(pass) = &setups.pass {
                let target = pass.target_order(&state.mods.borrow());
                apply_order(state, target)?;
            }
            save_setups(state, &setups)
        })();
        ui.global::<PassState>().set_error(result.err().unwrap_or_default().into());
        refresh(ui, state);
    }
    {
        let (weak, state) = (ui.as_weak(), state.clone());
        ps.on_start(move |from_current| {
            let Some(ui) = weak.upgrade() else { return };
            act(&ui, &state, |state, pass| {
                let baseline =
                    if from_current { super::order_view::current_order(&state.mods.borrow()) } else { vec![] };
                *pass = Some(Pass { baseline, batches: vec![] });
                Ok(())
            });
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
            act(&ui, &state, |_, pass| pass.as_mut().ok_or("Start a pass first.")?.begin(&name, picked));
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
            act(&ui, &state, |state, pass| {
                let order = super::order_view::current_order(&state.mods.borrow());
                pass.as_mut().ok_or("There is no pass.")?.mark(good, &order, &notes)
            });
        });
    }
    {
        let (weak, state) = (ui.as_weak(), state.clone());
        ps.on_end(move || {
            let Some(ui) = weak.upgrade() else { return };
            // Ending keeps the game as it is now; nothing in SMF changes.
            let result = (|| {
                let mut setups = state.setups.borrow().clone()?;
                setups.pass = None;
                save_setups(&state, &setups)
            })();
            ui.global::<PassState>().set_error(result.err().unwrap_or_default().into());
            refresh(&ui, &state);
        });
    }
}

/// Fills the deploy line, the setup label, the Setups sheet and the pass view.
pub fn refresh_extras(ui: &AppWindow, state: &State) {
    let g = ui.global::<Ui>();
    let game = state.game.borrow().clone();
    let current = game.as_ref().and_then(|g| state.smf.deploy_fingerprint(g).ok());
    let (running, last) = {
        let p = state.deploy.lock().expect("deploy progress lock");
        (p.running, p.last_line.clone())
    };
    let blocker = if running { None } else { play::blocked(&crate::platform::running(&play::BLOCKERS)) };
    let s = play::status(running, &last, &state.record.borrow(), current.as_deref(), blocker.as_deref());
    g.set_deploy_status(s.text.into());
    g.set_deploy_tone(s.tone.into());
    g.set_can_deploy(s.can_deploy && game.is_some());
    g.set_can_play(s.can_play && game.is_some());
    g.set_has_log(s.has_log);

    // Setups.
    let ss = ui.global::<SetupsState>();
    let now = game.as_ref().and_then(|g| state.smf.current_setup(g).ok());
    match &*state.setups.borrow() {
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
            ss.set_unreadable(e.clone().into());
            g.set_setup_label("Setups".into());
        }
    }

    // The pass view.
    let ps = ui.global::<PassState>();
    let mods = state.mods.borrow();
    let name_of = |id: &str| mods.iter().find(|m| m.id == id).map_or(id.to_string(), |m| m.name.clone());
    let pass = state.setups.borrow().as_ref().ok().and_then(|s| s.pass.clone());
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
                    .filter(|b| b.status == crate::setups::BatchStatus::Bad && b.mods.contains(&m.id))
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
