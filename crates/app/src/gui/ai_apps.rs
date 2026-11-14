//! Settings' AI tools card: each AI app (Claude Code, Gemini CLI, Codex CLI)
//! found, connected through its own command, checked, and the last time it
//! really reached Vaexil Mod Manager. The work is in `vmm_engine::ai_apps`;
//! the app's own commands run off the window's thread.

use std::cell::RefCell;
use std::rc::Rc;

use slint::{ComponentHandle, Model, ModelRc, VecModel};
use vmm_engine::Engine;
use vmm_engine::ai_apps::{self as ai, AiApp, Check};

use super::settings::in_background;
use crate::{AiAppRow, AppWindow, SettingsState, Ui, cli_path};

/// How often the "last reached" line is refreshed while Settings is open.
const WATCH_EVERY: std::time::Duration = std::time::Duration::from_secs(4);

thread_local! {
    static ROWS: Rc<VecModel<AiAppRow>> = Rc::new(VecModel::default());
    /// The last check of each app, by its place in `AiApp::ALL`.
    static CHECKS: RefCell<[Option<Check>; 3]> = const { RefCell::new([None, None, None]) };
    static WATCH: slint::Timer = slint::Timer::default();
}

fn rows() -> Rc<VecModel<AiAppRow>> {
    ROWS.with(Rc::clone)
}

fn checked(i: usize) -> Option<Check> {
    CHECKS.with(|c| c.borrow()[i].clone())
}

/// The row for `app` from what is known now, keeping its note.
fn row(engine: &Engine, app: AiApp, check: Option<&Check>, keep: Option<&AiAppRow>) -> AiAppRow {
    let found = ai::find(app);
    let detail = match &found {
        Some(f) if f.desktop_copy => format!("Found inside the Claude desktop app: {}", f.path.display()),
        Some(f) => format!("Found: {}", f.path.display()),
        None => format!("Not installed. Get it from {}", app.install_page()),
    };
    let (badge, tone, connected) = match (&found, check) {
        (None, _) => ("Not installed", "off", false),
        (Some(_), None) => ("Not checked", "off", false),
        (Some(_), Some(Check::Connected)) => ("Connected", "ok", true),
        (Some(_), Some(Check::Listed)) => ("Added", "ok", true),
        (Some(_), Some(Check::NeedsTrust)) => ("Added, needs a trusted folder", "warn", true),
        (Some(_), Some(Check::NotConnected)) => ("Not connected", "off", false),
        (Some(_), Some(Check::Failed(_))) => ("Failed", "bad", true),
    };
    let last_used = engine
        .last_client(app)
        .map(|l| {
            format!("Last reached Vaexil Mod Manager: {}, {} UTC.", l.client, vmm_engine::events::timestamp(l.at_ms))
        })
        .unwrap_or_default();
    let (note, note_error) = match (keep, check) {
        (Some(k), _) if !k.note.is_empty() => (k.note.to_string(), k.note_error),
        (_, Some(Check::Failed(why))) => (why.clone(), true),
        (_, Some(Check::NeedsTrust)) => (
            "Gemini CLI uses it only in folders you have trusted in Gemini CLI. It asks when you start it in a new folder."
                .to_string(),
            false,
        ),
        (_, Some(Check::Listed)) => {
            ("Codex CLI doesn't start it to check; a test request shows that it works.".to_string(), false)
        }
        _ => (String::new(), false),
    };
    AiAppRow {
        name: app.name().into(),
        found: found.is_some(),
        detail: detail.into(),
        badge: badge.into(),
        tone: tone.into(),
        connected,
        can_test: found.as_ref().is_some_and(|f| ai::test_command(app, f).is_some()),
        busy: false,
        last_used: last_used.into(),
        note: note.into(),
        note_error,
    }
}

/// Puts the rows into Settings and checks each app that hasn't been checked.
pub fn show(ui: &AppWindow, engine: &Engine) {
    let ss = ui.global::<SettingsState>();
    let model = rows();
    ss.set_ai_apps(ModelRc::from(model.clone()));
    ss.set_ai_test_request(ai::TEST_REQUEST.into());
    ss.set_ai_confirming(-1);
    let fresh: Vec<AiAppRow> =
        AiApp::ALL.iter().enumerate().map(|(i, app)| row(engine, *app, checked(i).as_ref(), None)).collect();
    model.set_vec(fresh);
    for (i, app) in AiApp::ALL.iter().enumerate() {
        if checked(i).is_none() && ai::find(*app).is_some() {
            check(ui, i);
        }
    }
}

/// Shows `note` on row `i`, and whether it is working.
fn set_note(i: usize, note: &str, error: bool, busy: bool) {
    let model = rows();
    if let Some(mut r) = model.row_data(i) {
        r.note = note.into();
        r.note_error = error;
        r.busy = busy;
        if busy {
            r.badge = "Working".into();
            r.tone = "busy".into();
        }
        model.set_row_data(i, r);
    }
}

/// Rebuilds row `i` from its last check, with `note`.
fn refresh(i: usize, note: &str, error: bool) {
    let engine = Engine::open();
    let model = rows();
    let keep = AiAppRow { note: note.into(), note_error: error, ..Default::default() };
    model.set_row_data(i, row(&engine, AiApp::ALL[i], checked(i).as_ref(), Some(&keep)));
}

/// Asks app `i` whether it has Vaexil Mod Manager, off the window's thread.
fn check(ui: &AppWindow, i: usize) {
    let app = AiApp::ALL[i];
    set_note(i, &format!("Asking {}…", app.name()), false, true);
    in_background(
        ui,
        move |_| Engine::open().check_ai_app(app),
        move |_, r| match r {
            Ok(c) => {
                CHECKS.with(|s| s.borrow_mut()[i] = Some(c));
                refresh(i, "", false);
            }
            Err(e) => refresh(i, &e.message, true),
        },
    );
}

pub fn wire(ui: &AppWindow) {
    let ss = ui.global::<SettingsState>();
    {
        let weak = ui.as_weak();
        ss.on_ai_check(move |i| {
            if let Some(ui) = weak.upgrade() {
                check(&ui, i as usize);
            }
        });
    }
    {
        let weak = ui.as_weak();
        ss.on_ai_connect(move |i| {
            let Some(ui) = weak.upgrade() else { return };
            let app = AiApp::ALL[i as usize];
            let (Some(found), Some(exe)) = (ai::find(app), cli_path::cli_exe()) else {
                set_note(
                    i as usize,
                    "vmm.exe isn't beside the app, so there is nothing to connect. Reinstall Vaexil Mod Manager.",
                    true,
                    false,
                );
                return;
            };
            let ss = ui.global::<SettingsState>();
            ss.set_ai_confirm_command(ai::connect_command(app, &found, &exe).display().into());
            ss.set_ai_confirm_remove(false);
            ss.set_ai_confirming(i);
        });
    }
    {
        let weak = ui.as_weak();
        ss.on_ai_disconnect(move |i| {
            let Some(ui) = weak.upgrade() else { return };
            let app = AiApp::ALL[i as usize];
            let Some(found) = ai::find(app) else { return };
            let ss = ui.global::<SettingsState>();
            ss.set_ai_confirm_command(ai::disconnect_command(app, &found).display().into());
            ss.set_ai_confirm_remove(true);
            ss.set_ai_confirming(i);
        });
    }
    {
        let weak = ui.as_weak();
        ss.on_ai_cancel(move || {
            if let Some(ui) = weak.upgrade() {
                ui.global::<SettingsState>().set_ai_confirming(-1);
            }
        });
    }
    {
        let weak = ui.as_weak();
        ss.on_ai_confirm(move || {
            let Some(ui) = weak.upgrade() else { return };
            let ss = ui.global::<SettingsState>();
            let (i, remove) = (ss.get_ai_confirming(), ss.get_ai_confirm_remove());
            ss.set_ai_confirming(-1);
            let Ok(i) = usize::try_from(i) else { return };
            let app = AiApp::ALL[i];
            let exe = cli_path::cli_exe().unwrap_or_default();
            set_note(i, if remove { "Removing…" } else { "Connecting…" }, false, true);
            in_background(
                &ui,
                move |_| {
                    let e = Engine::open();
                    if remove { e.disconnect_ai_app(app) } else { e.connect_ai_app(app, &exe) }
                },
                move |ui, r| {
                    // Check again, so the badge says what the app now says.
                    CHECKS.with(|s| s.borrow_mut()[i] = None);
                    match r {
                        Ok(m) => refresh(i, &m, false),
                        Err(e) => refresh(i, &e.message, true),
                    }
                    let note = rows().row_data(i).map(|r| (r.note.to_string(), r.note_error));
                    check(ui, i);
                    if let Some((n, err)) = note {
                        set_note(i, &n, err, true);
                    }
                },
            );
        });
    }
    {
        let weak = ui.as_weak();
        ss.on_ai_test(move |i| {
            let Some(ui) = weak.upgrade() else { return };
            let i = i as usize;
            let app = AiApp::ALL[i];
            set_note(i, &format!("Asking {} once; this uses your plan or credits…", app.name()), false, true);
            in_background(
                &ui,
                move |_| Engine::open().test_ai_app(app),
                move |_, r| match r {
                    Ok(answer) => {
                        let short: String = answer.chars().take(400).collect();
                        refresh(i, &format!("{} answered: {short}", app.name()), false);
                    }
                    Err(e) => refresh(i, &e.message, true),
                },
            );
        });
    }
    ss.on_ai_sent_test(move |i| {
        let app = AiApp::ALL[i as usize];
        set_note(
            i as usize,
            &format!(
                "Copied. Paste it into a new {} session; the green line updates when it reaches Vaexil Mod Manager.",
                app.name()
            ),
            false,
            false,
        );
    });
    // While Settings is open, the "last reached" lines follow the log, so a
    // test request sent from the AI app shows up here.
    let weak = ui.as_weak();
    WATCH.with(|t| {
        t.start(slint::TimerMode::Repeated, WATCH_EVERY, move || {
            let Some(ui) = weak.upgrade() else { return };
            if ui.global::<Ui>().get_sheet() != "settings" {
                return;
            }
            let engine = Engine::open();
            let model = rows();
            for (i, app) in AiApp::ALL.iter().enumerate() {
                let Some(mut r) = model.row_data(i) else { continue };
                let last = engine
                    .last_client(*app)
                    .map(|l| {
                        format!(
                            "Last reached Vaexil Mod Manager: {}, {} UTC.",
                            l.client,
                            vmm_engine::events::timestamp(l.at_ms)
                        )
                    })
                    .unwrap_or_default();
                if r.last_used != last.as_str() {
                    r.last_used = last.into();
                    model.set_row_data(i, r);
                }
            }
        });
    });
}
