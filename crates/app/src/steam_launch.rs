//! `vaexil-mod-manager --steam-play -- <game command>`: what Steam's Play
//! button runs once Settings has connected it. With the mods deployed, the
//! game starts at once and no window shows. With changes since the last
//! deploy, a small window offers to deploy first or play the build already
//! in the game. The game is started from Steam's own command, never through
//! Steam again, so Play can't loop.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use slint::ComponentHandle;
use vmm_engine::Engine;
use vmm_engine::host;

use crate::{DeployWindow, PlayChooser, show_message};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Choice {
    Deploy,
    PlayNow,
    Cancel,
}

/// Why the choice is offered, in words, for the window and Steam alike.
pub fn reason(engine: &Engine) -> String {
    if engine.record().at_ms.is_none() {
        "Vaexil Mod Manager hasn't deployed yet, so it can't tell whether the game has your current mods. Deploy \
         first, or play the game as it is."
            .into()
    } else {
        "Your mods changed since the last deploy. Deploy first (SMF's Deploy.exe takes a few minutes), or play the \
         build already in the game, without these changes."
            .into()
    }
}

fn choose(reason: &str, can_deploy: bool) -> Choice {
    let Ok(w) = PlayChooser::new() else { return Choice::Cancel };
    w.set_reason(reason.into());
    w.set_can_deploy(can_deploy);
    let picked = Rc::new(std::cell::Cell::new(Choice::Cancel));
    for (choice, setter) in [
        (Choice::Deploy, PlayChooser::on_deploy_then_play as fn(&PlayChooser, _)),
        (Choice::PlayNow, PlayChooser::on_play_now),
        (Choice::Cancel, PlayChooser::on_cancel),
    ] {
        let (weak, picked) = (w.as_weak(), picked.clone());
        setter(&w, move || {
            picked.set(choice);
            if let Some(w) = weak.upgrade() {
                let _ = w.hide();
            }
        });
    }
    let _ = w.run();
    picked.get()
}

/// Runs a deploy with a progress window. Returns why it didn't succeed.
fn deploy(engine: &Rc<Engine>, game: &vmm_core::game::GameInstall) -> Result<(), String> {
    engine.start_deploy(game, false).map_err(|e| e.message)?;
    let w = DeployWindow::new().map_err(|e| e.to_string())?;
    // A deploy can't be stopped halfway, so the window stays until it ends.
    w.window().on_close_requested(|| slint::CloseRequestResponse::KeepWindowShown);
    let outcome: Rc<RefCell<Option<Result<(), String>>>> = Rc::default();
    let timer = slint::Timer::default();
    {
        let (weak, engine, game, outcome) = (w.as_weak(), engine.clone(), game.clone(), outcome.clone());
        timer.start(slint::TimerMode::Repeated, Duration::from_millis(300), move || {
            let (running, line) = engine.deploy_progress();
            let Some(w) = weak.upgrade() else { return };
            w.set_line(line.into());
            if running {
                return;
            }
            if let Some(problem) = engine.finish_deploy(Some(&game)) {
                let ok = engine.record().success == Some(true);
                *outcome.borrow_mut() = Some(match (problem, ok) {
                    (Some(e), _) => Err(e.message),
                    (None, true) => Ok(()),
                    (None, false) => Err("SMF's Deploy.exe reported a failure; its log has the details.".into()),
                });
                let _ = w.hide();
            }
        });
    }
    let _ = w.run();
    drop(timer);
    outcome.take().unwrap_or_else(|| Err("The deploy didn't report back.".into()))
}

fn play(command: &[String]) -> i32 {
    match host::run_command(command) {
        Ok(_) => 0,
        Err(e) => {
            show_message("Couldn't start HITMAN", &e.message);
            1
        }
    }
}

pub fn run(args: &[String]) -> i32 {
    let command: Vec<String> = match args.iter().position(|a| a == "--") {
        Some(i) => args[i + 1..].to_vec(),
        None => Vec::new(),
    };
    let engine = Rc::new(Engine::open());
    // Without the game found, there is nothing to check: start it as Steam asked.
    let Some(game) = engine.installs().into_iter().next() else { return play(&command) };
    if engine.up_to_date(&game) == Some(true) {
        return play(&command);
    }
    let blocked = engine.blocker().map(|e| e.message);
    let reason = match &blocked {
        Some(why) => format!("{} {why}", reason(&engine)),
        None => reason(&engine),
    };
    match choose(&reason, blocked.is_none()) {
        Choice::Cancel => 0,
        Choice::PlayNow => play(&command),
        Choice::Deploy => match deploy(&engine, &game) {
            Ok(()) => play(&command),
            Err(why) => {
                let again =
                    format!("The deploy didn't succeed: {why} You can still play the build already in the game.");
                match choose(&again, false) {
                    Choice::PlayNow => play(&command),
                    _ => 0,
                }
            }
        },
    }
}
