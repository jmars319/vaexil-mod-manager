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
fn show(ui: &AppWindow, state: &State) {
    let ss = ui.global::<SettingsState>();
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
