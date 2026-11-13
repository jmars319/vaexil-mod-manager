//! The Settings sheet. Today it holds one thing: putting `vmm.exe` on the
//! user's PATH, which happens only after the sheet has shown exactly what
//! will change and the user has confirmed it.

use slint::ComponentHandle;

use crate::{AppWindow, SettingsState, Ui, cli_path};

/// Reads where `vmm.exe` is and whether it is on PATH into the sheet.
fn show(ui: &AppWindow) {
    let ss = ui.global::<SettingsState>();
    let exe = cli_path::cli_exe();
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

fn change(ui: &AppWindow, add: bool) {
    let ss = ui.global::<SettingsState>();
    ss.set_confirming(false);
    let Some(folder) = cli_path::cli_exe().and_then(|e| e.parent().map(|f| f.to_path_buf())) else {
        show(ui);
        return;
    };
    let (note, error) = match cli_path::change_user_path(&folder, add) {
        Ok(()) if add => ("Added. Open a new terminal and run `vmm status`.".to_string(), false),
        Ok(()) => ("Taken out of your PATH.".to_string(), false),
        Err(e) => (format!("{e} PATH was not changed."), true),
    };
    show(ui);
    ss.set_note(note.into());
    ss.set_note_error(error);
}

pub fn wire_settings(ui: &AppWindow) {
    let ss = ui.global::<SettingsState>();
    {
        let weak = ui.as_weak();
        ui.global::<Ui>().on_show_settings(move || {
            let Some(ui) = weak.upgrade() else { return };
            let ss = ui.global::<SettingsState>();
            ss.set_confirming(false);
            ss.set_note("".into());
            ss.set_note_error(false);
            show(&ui);
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
        let weak = ui.as_weak();
        ss.on_confirm_add(move || {
            if let Some(ui) = weak.upgrade() {
                change(&ui, true);
            }
        });
    }
    {
        let weak = ui.as_weak();
        ss.on_remove_from_path(move || {
            if let Some(ui) = weak.upgrade() {
                change(&ui, false);
            }
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
