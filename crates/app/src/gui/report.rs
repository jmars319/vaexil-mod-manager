//! The "Last deploy" sheet: the engine's deploy report, in words.

use std::rc::Rc;

use slint::{ComponentHandle, ModelRc, VecModel};
use vmm_core::schema::{DeployReport, DeployResult};

use super::State;
use crate::{AppWindow, ReportState, ReportWarning, Ui};

/// The sheet shows this many warning groups and errors; the rest are counted.
const SHOWN_WARNINGS: usize = 6;
const SHOWN_ERRORS: usize = 5;

fn outcome(r: &DeployReport) -> String {
    let mut t = match r.result {
        DeployResult::Succeeded => "The last deploy succeeded".to_string(),
        DeployResult::Failed => match r.exit_code {
            Some(c) => format!("The last deploy failed (exit code {c})"),
            None => "The last deploy failed".to_string(),
        },
        DeployResult::DidNotRun => "The last deploy didn't run".to_string(),
    };
    if let Some(ms) = r.duration_ms {
        t += &format!(", in {}m {}s", ms / 60_000, ms / 1000 % 60);
    }
    t + "."
}

fn fill(ui: &AppWindow, report: Option<DeployReport>) {
    let rs = ui.global::<ReportState>();
    let Some(r) = report else {
        rs.set_outcome("No deploy has finished yet.".into());
        rs.set_failed(false);
        rs.set_stages("".into());
        rs.set_warnings(ModelRc::default());
        rs.set_errors(ModelRc::default());
        rs.set_has_log(false);
        rs.set_more("".into());
        rs.set_meaning("".into());
        return;
    };
    rs.set_outcome(outcome(&r).into());
    rs.set_failed(r.result != DeployResult::Succeeded);
    let mods = r.stages.iter().filter(|s| s.mod_id.is_some()).count();
    rs.set_stages(
        format!("{} stages, {mods} of them deploying a mod. {} lines in the log.", r.stages.len(), r.log_lines).into(),
    );
    let warnings: Vec<ReportWarning> = r
        .warnings
        .iter()
        .take(SHOWN_WARNINGS)
        .map(|w| {
            let times = if w.count == 1 { "once".to_string() } else { format!("{} times", w.count) };
            ReportWarning {
                title: match (&w.mod_name, w.examples.first()) {
                    (Some(m), _) => format!("{m}: {times}"),
                    (None, Some(e)) => format!("{times}: {e}"),
                    (None, None) => format!("{}: {times}", w.code),
                }
                .into(),
            }
        })
        .collect();
    rs.set_warnings(ModelRc::from(Rc::new(VecModel::from(warnings))));
    let mut meaning: Vec<&str> = Vec::new();
    for w in r.warnings.iter().take(SHOWN_WARNINGS) {
        if !meaning.contains(&w.detail.as_str()) {
            meaning.push(&w.detail);
        }
    }
    rs.set_meaning(meaning.join(" ").into());
    let errors: Vec<slint::SharedString> = r.errors.iter().take(SHOWN_ERRORS).map(|e| e.as_str().into()).collect();
    let (more_w, more_e) =
        (r.warnings.len().saturating_sub(SHOWN_WARNINGS), r.errors.len().saturating_sub(SHOWN_ERRORS));
    let more = match (more_w, more_e) {
        (0, 0) => String::new(),
        (w, e) => format!(
            "Not shown: {w} more kind(s) of warning and {e} more error(s). The log has them all, and \u{201c}vmm deploy-report\u{201d} lists them."
        ),
    };
    rs.set_more(more.into());
    rs.set_errors(ModelRc::from(Rc::new(VecModel::from(errors))));
    rs.set_has_log(r.log.is_some());
}

pub fn wire_report(ui: &AppWindow, state: &Rc<State>) {
    {
        let (weak, state) = (ui.as_weak(), state.clone());
        ui.global::<Ui>().on_show_report(move || {
            let Some(ui) = weak.upgrade() else { return };
            fill(&ui, state.engine.deploy_report());
            ui.global::<Ui>().set_sheet("report".into());
        });
    }
    {
        let weak = ui.as_weak();
        ui.global::<ReportState>().on_open_log(move || {
            if let Some(ui) = weak.upgrade() {
                ui.global::<Ui>().invoke_open_log();
            }
        });
    }
    {
        let weak = ui.as_weak();
        ui.global::<ReportState>().on_close(move || {
            if let Some(ui) = weak.upgrade() {
                ui.global::<Ui>().set_sheet("".into());
            }
        });
    }
}
