//! The Settings sheet: putting `vmm.exe` on the user's PATH (only after the
//! sheet has shown exactly what will change and the user has confirmed it),
//! "Let AI tools make changes", and each AI app's own command for adding
//! `vmm mcp` as a server.

use std::path::Path;
use std::rc::Rc;

use slint::ComponentHandle;

use super::State;
use crate::{AppWindow, SettingsState, Ui, cli_path};

/// The server's name in the AI apps' settings.
const SERVER_NAME: &str = "vaexil-mod-manager";

/// Each AI app's own command for adding `vmm mcp` as a server, for the user
/// to run. Claude Code and Gemini CLI get user scope (every project), not
/// their default of the current project.
fn connect_commands(exe: &Path) -> [String; 3] {
    let exe = exe.display();
    [
        format!("claude mcp add --scope user {SERVER_NAME} -- \"{exe}\" mcp"),
        format!("gemini mcp add --scope user {SERVER_NAME} \"{exe}\" mcp"),
        format!("codex mcp add {SERVER_NAME} -- \"{exe}\" mcp"),
    ]
}

/// Reads where `vmm.exe` is and whether it is on PATH into the sheet.
/// Steam's Play button as it stands, into the sheet.
fn show_steam(ui: &AppWindow, state: &State) {
    let ss = ui.global::<SettingsState>();
    let exe = std::env::current_exe().unwrap_or_default();
    let v = state.engine.steam_play_view(&exe);
    ss.set_steam_account(v.account.unwrap_or_default().into());
    ss.set_steam_current(v.current.into());
    ss.set_steam_proposed(v.proposed.into());
    ss.set_steam_connected(v.connected);
    ss.set_steam_running(v.steam_running);
    ss.set_steam_problem(v.problem.unwrap_or_default().into());
}

/// SMF's version and the last update check, into the sheet.
fn show_smf(ui: &AppWindow, state: &State) {
    let ss = ui.global::<SettingsState>();
    let game = state.game.borrow().clone();
    let s = state.engine.smf_update_status(game.as_ref());
    let installed = s.installed.clone();
    let checked = s.checked_at_ms.map(|t| format!(" Checked {} UTC.", vmm_engine::events::timestamp(t)));
    let text = match (&installed, &s.latest, s.held) {
        (None, _, _) => {
            "The installed SMF isn't a release this app can name, so it can't be updated from here.".to_string()
        }
        (Some(i), Some(l), false) => {
            format!("Installed: {i}. SMF {l} is available ({} MB).{}", s.size_bytes >> 20, checked.unwrap_or_default())
        }
        (Some(i), Some(l), true) => format!(
            "Installed: {i}. SMF {l} is out, but it can't be applied automatically (its release notes say so, or this SMF is too old to check them all); update it by hand.{}",
            checked.unwrap_or_default()
        ),
        (Some(i), None, _) => match checked {
            Some(c) => format!("Installed: {i}, the newest.{c}"),
            None => format!("Installed: {i}. Not checked yet."),
        },
    };
    ss.set_smf_status(text.into());
    ss.set_smf_can_update(installed.is_some() && s.latest.is_some() && !s.held);
}

fn show(ui: &AppWindow, state: &State) {
    let ss = ui.global::<SettingsState>();
    show_steam(ui, state);
    show_smf(ui, state);
    ss.set_ai_changes(state.engine.ai_changes_allowed());
    let exe = cli_path::cli_exe();
    let [claude, gemini, codex] = exe.as_deref().map(connect_commands).unwrap_or_default();
    ss.set_claude_command(claude.into());
    ss.set_gemini_command(gemini.into());
    ss.set_codex_command(codex.into());
    let folder = exe.as_ref().and_then(|e| e.parent()).map(|f| f.to_path_buf());
    ss.set_cli_exe(exe.map(|e| e.display().to_string()).unwrap_or_default().into());
    ss.set_cli_folder(folder.as_ref().map(|f| f.display().to_string()).unwrap_or_default().into());
    let on_path = match &folder {
        Some(f) => cli_path::on_user_path(f),
        None => Ok(false),
    };
    match on_path {
        Ok(on) => ss.set_on_path(on),
        Err(e) => {
            ss.set_on_path(false);
            ss.set_note(e.into());
            ss.set_note_error(true);
        }
    }
}

fn change(ui: &AppWindow, state: &State, add: bool) {
    let ss = ui.global::<SettingsState>();
    ss.set_confirming(false);
    let Some(folder) = cli_path::cli_exe().and_then(|e| e.parent().map(|f| f.to_path_buf())) else {
        show(ui, state);
        return;
    };
    let (note, error) = match cli_path::change_user_path(&folder, add) {
        Ok(()) if add => ("Added. Open a new terminal and run `vmm status`.".to_string(), false),
        Ok(()) => ("Taken out of your PATH.".to_string(), false),
        Err(e) => (format!("{e} PATH was not changed."), true),
    };
    show(ui, state);
    ss.set_note(note.into());
    ss.set_note_error(error);
}

pub fn wire_settings(ui: &AppWindow, state: &Rc<State>) {
    let ss = ui.global::<SettingsState>();
    {
        let (weak, state) = (ui.as_weak(), state.clone());
        ui.global::<Ui>().on_show_settings(move || {
            let Some(ui) = weak.upgrade() else { return };
            let ss = ui.global::<SettingsState>();
            ss.set_confirming(false);
            ss.set_note("".into());
            ss.set_note_error(false);
            ss.set_ai_note("".into());
            ss.set_diagnostics_note("".into());
            ss.set_steam_confirming(false);
            ss.set_steam_note("".into());
            show(&ui, &state);
            ui.global::<Ui>().set_sheet("settings".into());
        });
    }
    {
        let weak = ui.as_weak();
        ss.on_add_to_path(move || {
            if let Some(ui) = weak.upgrade() {
                let ss = ui.global::<SettingsState>();
                ss.set_note("".into());
                ss.set_confirming(true);
            }
        });
    }
    {
        let (weak, state) = (ui.as_weak(), state.clone());
        ss.on_confirm_add(move || {
            if let Some(ui) = weak.upgrade() {
                change(&ui, &state, true);
            }
        });
    }
    {
        let (weak, state) = (ui.as_weak(), state.clone());
        ss.on_remove_from_path(move || {
            if let Some(ui) = weak.upgrade() {
                change(&ui, &state, false);
            }
        });
    }
    {
        let (weak, state) = (ui.as_weak(), state.clone());
        ss.on_set_ai_changes(move |on| {
            let Some(ui) = weak.upgrade() else { return };
            let ss = ui.global::<SettingsState>();
            let note = match state.engine.set_ai_changes(on) {
                Ok(()) => String::new(),
                Err(e) => e.message,
            };
            ss.set_ai_note(note.into());
            ss.set_ai_changes(state.engine.ai_changes_allowed());
        });
    }
    {
        let (weak, state) = (ui.as_weak(), state.clone());
        ss.on_make_diagnostics(move || {
            let Some(ui) = weak.upgrade() else { return };
            let ss = ui.global::<SettingsState>();
            let game = state.game.borrow().clone();
            let d = state.engine.diagnose(game.as_ref());
            ss.set_diagnostics(serde_json::to_string_pretty(&d).unwrap_or_default().into());
            ss.set_diagnostics_note("Copied.".into());
        });
    }
    {
        let (weak, state) = (ui.as_weak(), state.clone());
        ss.on_steam_connect(move || {
            let Some(ui) = weak.upgrade() else { return };
            show_steam(&ui, &state);
            let ss = ui.global::<SettingsState>();
            ss.set_steam_note("".into());
            ss.set_steam_confirming(true);
        });
    }
    {
        let weak = ui.as_weak();
        ss.on_steam_cancel(move || {
            if let Some(ui) = weak.upgrade() {
                ui.global::<SettingsState>().set_steam_confirming(false);
            }
        });
    }
    for connect in [true, false] {
        let (weak, state) = (ui.as_weak(), state.clone());
        let handler = move || {
            let Some(ui) = weak.upgrade() else { return };
            let r = if connect {
                state.engine.connect_steam_play(&std::env::current_exe().unwrap_or_default())
            } else {
                state.engine.disconnect_steam_play()
            };
            let ss = ui.global::<SettingsState>();
            ss.set_steam_confirming(false);
            show_steam(&ui, &state);
            match r {
                Ok(m) => {
                    ss.set_steam_note(m.into());
                    ss.set_steam_note_error(false);
                }
                Err(e) => {
                    ss.set_steam_note(e.message.into());
                    ss.set_steam_note_error(true);
                }
            }
        };
        if connect {
            ss.on_steam_confirm(handler);
        } else {
            ss.on_steam_disconnect(handler);
        }
    }
    wire_smf(ui, state);
    {
        let weak = ui.as_weak();
        ss.on_cancel_add(move || {
            if let Some(ui) = weak.upgrade() {
                ui.global::<SettingsState>().set_confirming(false);
            }
        });
    }
    {
        let weak = ui.as_weak();
        ss.on_close(move || {
            if let Some(ui) = weak.upgrade() {
                ui.global::<SettingsState>().set_confirming(false);
                ui.global::<Ui>().set_sheet("".into());
            }
        });
    }
}

type Slot<T> = std::sync::Arc<std::sync::Mutex<Option<T>>>;

/// Runs `work` on a worker thread and hands its result to `done` on the
/// window's thread, polling every 300 ms; `line` shows progress meanwhile.
fn in_background<T: Send + 'static>(
    ui: &AppWindow,
    work: impl FnOnce(&dyn Fn(String)) -> T + Send + 'static,
    done: impl Fn(&AppWindow, T) + 'static,
) {
    let result: Slot<T> = Default::default();
    let line: std::sync::Arc<std::sync::Mutex<String>> = Default::default();
    {
        let (result, line) = (result.clone(), line.clone());
        std::thread::spawn(move || {
            let r = work(&|l| *line.lock().expect("progress lock") = l);
            *result.lock().expect("result lock") = Some(r);
        });
    }
    let timer = Rc::new(slint::Timer::default());
    let keep = timer.clone();
    let weak = ui.as_weak();
    timer.start(slint::TimerMode::Repeated, std::time::Duration::from_millis(300), move || {
        let Some(ui) = weak.upgrade() else { return };
        let progress = line.lock().expect("progress lock").clone();
        if !progress.is_empty() {
            ui.global::<SettingsState>().set_smf_note(progress.into());
        }
        if let Some(r) = result.lock().expect("result lock").take() {
            keep.stop();
            done(&ui, r);
        }
    });
}

/// Whether an SMF update is running in this window. Closing the window
/// would end the process mid-copy, so it stays open until the update ends.
static UPDATE_RUNNING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

fn wire_smf(ui: &AppWindow, state: &Rc<State>) {
    let ss = ui.global::<SettingsState>();
    {
        let weak = ui.as_weak();
        ui.window().on_close_requested(move || {
            if !UPDATE_RUNNING.load(std::sync::atomic::Ordering::SeqCst) {
                return slint::CloseRequestResponse::HideWindow;
            }
            if let Some(ui) = weak.upgrade() {
                ui.global::<Ui>().set_sheet("settings".into());
                let ss = ui.global::<SettingsState>();
                ss.set_smf_note("SMF is being updated. The window closes once the update has finished.".into());
                ss.set_smf_note_error(true);
            }
            slint::CloseRequestResponse::KeepWindowShown
        });
    }
    let note = |ui: &AppWindow, text: String, error: bool| {
        let ss = ui.global::<SettingsState>();
        ss.set_smf_note(text.into());
        ss.set_smf_note_error(error);
    };
    {
        let (weak, state) = (ui.as_weak(), state.clone());
        ss.on_smf_check(move || {
            let Some(ui) = weak.upgrade() else { return };
            let Some(game) = state.game.borrow().clone() else { return };
            let (Some(data), Some(installed)) =
                (state.engine.data_dir().map(|d| d.to_path_buf()), state.engine.smf_version(&game))
            else {
                note(
                    &ui,
                    "The installed SMF isn't a release this app can name, so there is nothing to compare.".into(),
                    true,
                );
                return;
            };
            ui.global::<SettingsState>().set_smf_busy(true);
            note(&ui, "Checking SMF's releases on GitHub…".into(), false);
            let state = state.clone();
            in_background(
                &ui,
                move |_| vmm_engine::smf_update::check(&data, &installed).map(|_| ()),
                move |ui, r| {
                    ui.global::<SettingsState>().set_smf_busy(false);
                    show_smf(ui, &state);
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
        ss.on_smf_update(move || {
            if let Some(ui) = weak.upgrade() {
                let ss = ui.global::<SettingsState>();
                ss.set_smf_note("".into());
                ss.set_smf_confirming(true);
            }
        });
    }
    {
        let weak = ui.as_weak();
        ss.on_smf_cancel(move || {
            if let Some(ui) = weak.upgrade() {
                ui.global::<SettingsState>().set_smf_confirming(false);
            }
        });
    }
    {
        let (weak, state) = (ui.as_weak(), state.clone());
        ss.on_smf_confirm(move || {
            let Some(ui) = weak.upgrade() else { return };
            let ss = ui.global::<SettingsState>();
            ss.set_smf_confirming(false);
            let Some(game) = state.game.borrow().clone() else { return };
            // The local checks here; the network on the worker thread.
            let base = match state.engine.smf_update_base(&game) {
                Ok(b) => b,
                Err(e) => {
                    note(&ui, e.message, true);
                    return;
                }
            };
            ss.set_smf_busy(true);
            UPDATE_RUNNING.store(true, std::sync::atomic::Ordering::SeqCst);
            note(&ui, "Asking SMF's GitHub page for the newest release…".into(), false);
            let state = state.clone();
            in_background(
                &ui,
                move |line| -> Result<String, vmm_core::error::VmmError> {
                    match vmm_engine::smf_update::plan(base)? {
                        Some(job) => vmm_engine::smf_update::run(&job, &mut |l| line(l)),
                        None => Ok("SMF is already the newest.".into()),
                    }
                },
                move |ui, r| {
                    ui.global::<SettingsState>().set_smf_busy(false);
                    UPDATE_RUNNING.store(false, std::sync::atomic::Ordering::SeqCst);
                    if let Some(game) = state.game.borrow().clone() {
                        state.engine.note_smf_update(&game, &r);
                    }
                    show_smf(ui, &state);
                    match r {
                        Ok(m) => note(ui, m, false),
                        Err(e) => note(ui, e.message, true),
                    }
                },
            );
        });
    }
}

/// Checks SMF's releases in the background when the last check is over a
/// day old, so Settings and the main window can say an update is out.
pub fn daily_smf_check(state: &State) {
    let Some(game) = state.game.borrow().clone() else { return };
    let (Some(data), Some(installed)) =
        (state.engine.data_dir().map(|d| d.to_path_buf()), state.engine.smf_version(&game))
    else {
        return;
    };
    let due = vmm_engine::smf_update::last_check(&data).is_none_or(|l| {
        l.installed != installed
            || vmm_engine::deploy::now_ms().saturating_sub(l.checked_at_ms) > vmm_engine::smf_update::CHECK_EVERY_MS
    });
    // Tests and development runs (a stand-in game) never go online by themselves.
    if due && vmm_engine::host::game_override().is_none() {
        std::thread::spawn(move || {
            let _ = vmm_engine::smf_update::check(&data, &installed);
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn connect_commands_quote_the_path_and_use_user_scope() {
        let [claude, gemini, codex] = connect_commands(Path::new(r"C:\Program Files\VMM\vmm.exe"));
        assert_eq!(claude, r#"claude mcp add --scope user vaexil-mod-manager -- "C:\Program Files\VMM\vmm.exe" mcp"#);
        assert_eq!(gemini, r#"gemini mcp add --scope user vaexil-mod-manager "C:\Program Files\VMM\vmm.exe" mcp"#);
        assert_eq!(codex, r#"codex mcp add vaexil-mod-manager -- "C:\Program Files\VMM\vmm.exe" mcp"#);
    }
}
