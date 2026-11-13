//! `vmm`, the command line of Vaexil Mod Manager (ADR 0003, section 3).
//!
//! Every command runs the same engine the window uses. With `--json` a
//! command prints exactly one JSON object on stdout, its result or an
//! `ErrorReport`, in the shapes `vmm schema` describes. Exit codes are stable:
//! 0 ok, 2 bad usage, 3 refused by a rule, 4 changed meanwhile, 5 the game or
//! a deploy is running, 10 unexpected. Nothing changes without `--yes`.

mod mcp;

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use serde::Serialize;
use serde_json::Value;
use vmm_core::backend::ModKind;
use vmm_core::error::{ErrorCode, VmmError};
use vmm_core::schema::{self, Change, ErrorCodeList, ErrorReport, LastDeploy, LogList, SCHEMA_VERSION};
use vmm_engine::Engine;
use vmm_engine::journal::Caller;

const USAGE: &str = "\
vmm - Vaexil Mod Manager's command line

Reading (never changes anything):
  vmm status                    the game, the framework, the mods and the last deploy
  vmm games                     every install of the game found
  vmm mods                      every mod: on or off, place in the order, type, options
  vmm mod <id>                  one mod
  vmm order                     the load order, broken rules and a suggested order
  vmm setups                    saved setups, and the compatibility pass under way
  vmm errors                    every error code, what it means and what to do
  vmm logs [--limit <n>]        the newest events (50 by default): every change,
                                who made it, and every failure
  vmm deploy-report             the last deploy: stages, warnings and errors
  vmm diagnose                  everything above in one object, for a bug report;
                                paths under your profile start with ~
  vmm schema [--out <folder>]   the JSON Schema of every --json result

Changing (plan first, then apply the plan):
  vmm plan enable <id>...       turn mods on
  vmm plan disable <id>...      turn mods off
  vmm plan options <id> [<value>...]
                                choose exactly these options of a mod
  vmm plan kind <id> <type|auto>
                                set the type shown for a mod (gameplay,
                                models-textures, sound, dialogue, ui, text,
                                other), or go back to the one its files suggest
  vmm plan order <id>...        a new load order of the mods that are on
  vmm plan auto-sort            sort the load order by the mods' own rules
  vmm plan use-setup <name>     switch to a saved setup
  vmm plan save-setup <name>    save the mods that are on as a setup
  vmm plan delete-setup <name>  delete a saved setup
  vmm plan start-pass [--from-current]
                                start a compatibility pass from every mod off,
                                or with the mods on now as its baseline
  vmm plan try-batch <id>... [--name <name>]
                                try a batch on top of the pass's baseline
  vmm plan mark-batch good|bad [<notes>...]
                                mark the batch being tried
  vmm plan end-pass             end the pass, keeping the mods as they are
                                (setup and pass steps are planned one at a time)
  vmm plan --file <path|->      changes as JSON: a list of changes, or
                                { \"changes\": [...] } (see `vmm schema`)
  vmm apply <plan-id> --yes     apply a plan, once, within 10 minutes
  vmm deploy --yes              run SMF's Deploy.exe and wait for it, then
                                show its report (refused while the game runs)

For AI tools:
  vmm mcp                       an MCP server on stdin and stdout (docs/ai.md).
                                It may read freely; it applies a change only
                                when Settings allows AI changes.

Options:
  --json                        print one JSON object (the result, or an error)
  --game <folder>               act on this game folder, not the first one found
  --yes                         confirm a change
  -h, --help                    this text
  --version                     the version
";

struct Args {
    json: bool,
    yes: bool,
    game: Option<PathBuf>,
    out: Option<PathBuf>,
    file: Option<String>,
    limit: Option<usize>,
    name: Option<String>,
    from_current: bool,
    words: Vec<String>,
}

fn usage(message: impl Into<String>) -> VmmError {
    VmmError::new(ErrorCode::Usage, message)
}

fn parse(raw: Vec<String>) -> Result<Args, VmmError> {
    let mut args = Args {
        json: false,
        yes: false,
        game: None,
        out: None,
        file: None,
        limit: None,
        name: None,
        from_current: false,
        words: vec![],
    };
    let mut it = raw.into_iter();
    let mut words_only = false;
    while let Some(a) = it.next() {
        if words_only {
            args.words.push(a);
            continue;
        }
        let (flag, inline) = match a.split_once('=') {
            Some((f, v)) if f.starts_with("--") => (f.to_string(), Some(v.to_string())),
            _ => (a.clone(), None),
        };
        let mut value =
            |name: &str| inline.clone().or_else(|| it.next()).ok_or_else(|| usage(format!("{name} needs a value.")));
        match flag.as_str() {
            "--" => words_only = true,
            // Flags that take no value refuse one, so --yes=false never confirms.
            "--json" | "--yes" if inline.is_some() => {
                return Err(usage(format!("{flag} takes no value. Nothing was changed.")));
            }
            "--json" => args.json = true,
            "--yes" | "-y" => args.yes = true,
            "-h" | "--help" => args.words.insert(0, "help".into()),
            "--version" => args.words.insert(0, "version".into()),
            "--game" => args.game = Some(PathBuf::from(value("--game")?)),
            "--out" => args.out = Some(PathBuf::from(value("--out")?)),
            "--file" => args.file = Some(value("--file")?),
            "--name" => args.name = Some(value("--name")?),
            "--from-current" => args.from_current = true,
            "--limit" => {
                let v = value("--limit")?;
                args.limit = Some(v.parse().map_err(|_| usage(format!("--limit needs a number, not {v}.")))?);
            }
            f if f.starts_with("--") => return Err(usage(format!("Unknown option {f}."))),
            _ => args.words.push(a),
        }
    }
    Ok(args)
}

/// A command's result: its JSON, and the same for people.
struct Out {
    json: Value,
    text: String,
}

fn out<T: Serialize>(value: &T, text: String) -> Out {
    Out { json: serde_json::to_value(value).expect("results serialize"), text }
}

fn main() -> ExitCode {
    let raw: Vec<String> = std::env::args().skip(1).collect();
    let crash_caller = if raw == ["mcp"] { "mcp" } else { "cli" };
    vmm_engine::crash::install(vmm_engine::host::data_dir(), crash_caller.into());
    // The MCP server owns stdout for the protocol, so it prints nothing else.
    if raw == ["mcp"] {
        return match mcp::serve() {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("{e}");
                ExitCode::from(10)
            }
        };
    }
    let json_wanted = raw.iter().any(|a| a == "--json");
    let result = parse(raw).and_then(|args| {
        let json = args.json;
        run(args).map(|o| (o, json))
    });
    let stdout = std::io::stdout();
    let mut stdout = stdout.lock();
    match result {
        Ok((o, json)) => {
            if json {
                let _ = writeln!(stdout, "{}", serde_json::to_string_pretty(&o.json).expect("json"));
            } else {
                let _ = write!(stdout, "{}", o.text);
                if !o.text.ends_with('\n') {
                    let _ = writeln!(stdout);
                }
            }
            ExitCode::SUCCESS
        }
        Err(e) => {
            let report = ErrorReport::from(&e);
            if json_wanted {
                let _ = writeln!(stdout, "{}", serde_json::to_string_pretty(&report).expect("json"));
            } else {
                eprintln!("{} [{}]\n{}", e.message, report.error.code, report.error.hint);
            }
            ExitCode::from(u8::try_from(e.code.exit_code()).unwrap_or(10))
        }
    }
}

fn run(args: Args) -> Result<Out, VmmError> {
    let words: Vec<&str> = args.words.iter().map(String::as_str).collect();
    let engine = || Engine::open_as(Caller::Cli);
    match words.as_slice() {
        [] | ["help"] => Ok(Out { json: serde_json::json!({ "usage": USAGE }), text: USAGE.into() }),
        ["version"] => Ok(Out {
            json: serde_json::json!({ "schemaVersion": SCHEMA_VERSION, "appVersion": env!("CARGO_PKG_VERSION") }),
            text: format!("vmm {} (schema {SCHEMA_VERSION})", env!("CARGO_PKG_VERSION")),
        }),
        ["errors"] => {
            let list = ErrorCodeList::current();
            let text = list
                .codes
                .iter()
                .map(|c| format!("{:<26} exit {:<2}  {}\n{:<36}{}\n", c.code, c.exit_code, c.meaning, "", c.hint))
                .collect();
            Ok(out(&list, text))
        }
        ["schema"] => write_schema(args.out.as_deref()),
        ["logs"] => {
            let list =
                LogList { schema_version: SCHEMA_VERSION, events: engine().recent_events(args.limit.unwrap_or(50)) };
            let text = if list.events.is_empty() {
                "Nothing logged yet.\n".into()
            } else {
                list.events.iter().map(log_line).collect()
            };
            Ok(out(&list, text))
        }
        ["deploy-report"] => {
            let last = LastDeploy { schema_version: SCHEMA_VERSION, report: engine().deploy_report() };
            let text = last.report.as_ref().map_or_else(|| "No deploy has finished yet.\n".into(), report_text);
            Ok(out(&last, text))
        }
        ["diagnose"] => {
            let e = engine();
            let game = match e.pick_game(args.game.as_deref()) {
                Ok(g) => Some(g),
                Err(err) if args.game.is_none() && err.code == ErrorCode::GameNotFound => None,
                Err(err) => return Err(err),
            };
            let d = e.diagnose(game.as_ref());
            let text = serde_json::to_string_pretty(&d).expect("json");
            Ok(out(&d, text))
        }
        ["games"] => {
            let list = engine().games();
            let text = if list.games.is_empty() {
                "No install of HITMAN World of Assassination was found.".into()
            } else {
                list.games.iter().map(|g| format!("{:<16} {}\n", g.store, g.folder)).collect()
            };
            Ok(out(&list, text))
        }
        ["status"] => {
            let e = engine();
            let game = match e.pick_game(args.game.as_deref()) {
                Ok(g) => Some(g),
                Err(err) if args.game.is_none() && err.code == ErrorCode::GameNotFound => None,
                Err(err) => return Err(err),
            };
            let s = e.status(game.as_ref());
            Ok(out(&s, status_text(&s)))
        }
        ["mods"] | ["mods", "list"] => {
            let e = engine();
            let list = e.mod_list(&e.pick_game(args.game.as_deref())?)?;
            let text = list.mods.iter().map(mod_line).collect();
            Ok(out(&list, text))
        }
        ["mod", id] | ["mod", "show", id] => {
            let e = engine();
            let d = e.mod_detail(&e.pick_game(args.game.as_deref())?, id)?;
            Ok(out(&d, mod_text(&d.mod_view)))
        }
        ["order"] | ["order", "show"] => {
            let e = engine();
            let r = e.order_report(&e.pick_game(args.game.as_deref())?)?;
            Ok(out(&r, order_text(&r)))
        }
        ["setups"] => {
            let e = engine();
            let game = e.pick_game(args.game.as_deref()).ok();
            let list = e.setup_list(game.as_ref())?;
            Ok(out(&list, setups_text(&list)))
        }
        ["deploy"] => {
            if !args.yes {
                return Err(usage(
                    "Deploying runs SMF's Deploy.exe, which rewrites the game's patch files. Run `vmm deploy --yes` to \
                     confirm. Nothing was deployed.",
                ));
            }
            let e = engine();
            let game = e.pick_game(args.game.as_deref())?;
            let show = !args.json;
            let status = e.deploy_and_wait(&game, &mut |line| {
                if show && !line.is_empty() {
                    eprintln!("{line}");
                }
            })?;
            let text = status.report.as_ref().map_or_else(|| "The deploy finished.\n".into(), report_text);
            Ok(out(&status, text))
        }
        ["plan", rest @ ..] => {
            let changes = changes_from(rest, args.file.as_deref(), args.name.as_deref(), args.from_current)?;
            let e = engine();
            let plan = e.plan(&e.pick_game(args.game.as_deref())?, changes)?;
            let mut text = format!("Plan {} (apply within 10 minutes):\n", plan.plan_id);
            for (i, s) in plan.steps.iter().enumerate() {
                let note = if s.changes_something { "" } else { " (already so)" };
                text += &format!("  {}. {}{note}\n", i + 1, s.description);
            }
            text += &format!("Apply it with: vmm apply {} --yes\n", plan.plan_id);
            Ok(out(&plan, text))
        }
        ["apply", id] => {
            if !args.yes {
                return Err(usage(format!(
                    "Applying a plan changes your mods. Run `vmm apply {id} --yes` to confirm. Nothing was changed."
                )));
            }
            let e = engine();
            // Without --game, the game the plan was made for.
            let game = match args.game.as_deref() {
                Some(folder) => e.pick_game(Some(folder))?,
                None => e.plan_game(id)?,
            };
            let r = e.apply_plan(&game, id, &Caller::Cli, None)?;
            let mut text = String::new();
            for s in &r.steps {
                let note = if s.changes_something { "" } else { " (already so)" };
                text += &format!("Done: {}{note}\n", s.description);
            }
            if let Some(p) = &r.journal_problem {
                text += &format!("{p}\n");
            }
            if r.needs_deploy {
                text += "Deploy in Vaexil Mod Manager for the game to use the changes.\n";
            }
            Ok(out(&r, text))
        }
        [w, ..] => Err(usage(format!("Unknown command \u{201c}{w}\u{201d}. Run `vmm help` for the commands."))),
    }
}

fn changes_from(
    words: &[&str],
    file: Option<&str>,
    name: Option<&str>,
    from_current: bool,
) -> Result<Vec<Change>, VmmError> {
    let ids = |rest: &[&str]| rest.iter().map(|s| s.to_string()).collect::<Vec<_>>();
    match (words, file) {
        ([], Some(path)) => {
            let mut text = String::new();
            let read = if path == "-" {
                std::io::stdin().read_to_string(&mut text).map(|_| ())
            } else {
                std::fs::read_to_string(path).map(|t| text = t)
            };
            read.map_err(|e| usage(format!("Couldn't read {path}: {e}")))?;
            let value: Value = serde_json::from_str(&text).map_err(|e| usage(format!("{path} isn't JSON: {e}")))?;
            let list = match value {
                Value::Object(mut o) => o.remove("changes").unwrap_or(Value::Null),
                v => v,
            };
            serde_json::from_value(list)
                .map_err(|e| usage(format!("{path} isn't a list of changes: {e}. Run `vmm schema` for the shape.")))
        }
        (_, Some(_)) => Err(usage("Give the changes either as words or with --file, not both.")),
        (["enable", rest @ ..], None) if !rest.is_empty() => {
            Ok(ids(rest).into_iter().map(|m| Change::SetEnabled { mod_id: m, enabled: true }).collect())
        }
        (["disable", rest @ ..], None) if !rest.is_empty() => {
            Ok(ids(rest).into_iter().map(|m| Change::SetEnabled { mod_id: m, enabled: false }).collect())
        }
        (["options", id, rest @ ..], None) => {
            Ok(vec![Change::SetOptions { mod_id: id.to_string(), chosen: ids(rest) }])
        }
        (["kind", id, kind], None) => {
            let kind = match *kind {
                "auto" => None,
                k => Some(ModKind::from_key(k).ok_or_else(|| {
                    usage(format!(
                        "\u{201c}{k}\u{201d} isn't a type. Use one of: {}, or auto.",
                        ModKind::ALL.map(|k| k.key()).join(", ")
                    ))
                })?),
            };
            Ok(vec![Change::SetKind { mod_id: id.to_string(), kind }])
        }
        (["order", rest @ ..], None) if !rest.is_empty() => Ok(vec![Change::Reorder { order: ids(rest) }]),
        (["auto-sort"], None) => Ok(vec![Change::AutoSort {}]),
        (["use-setup", rest @ ..], None) if !rest.is_empty() => Ok(vec![Change::UseSetup { name: rest.join(" ") }]),
        (["save-setup", rest @ ..], None) if !rest.is_empty() => Ok(vec![Change::SaveSetup { name: rest.join(" ") }]),
        (["delete-setup", rest @ ..], None) if !rest.is_empty() => {
            Ok(vec![Change::DeleteSetup { name: rest.join(" ") }])
        }
        (["start-pass"], None) => Ok(vec![Change::StartPass { from_current }]),
        (["try-batch", rest @ ..], None) if !rest.is_empty() => {
            Ok(vec![Change::TryBatch { name: name.unwrap_or_default().to_string(), mods: ids(rest) }])
        }
        (["mark-batch", verdict, notes @ ..], None) => {
            let good = match *verdict {
                "good" => true,
                "bad" => false,
                v => return Err(usage(format!("Mark the batch good or bad, not \u{201c}{v}\u{201d}."))),
            };
            Ok(vec![Change::MarkBatch { good, notes: notes.join(" ") }])
        }
        (["end-pass"], None) => Ok(vec![Change::EndPass {}]),
        _ => Err(usage("Plan what? Run `vmm help` for the kinds of change.")),
    }
}

fn write_schema(folder: Option<&Path>) -> Result<Out, VmmError> {
    let all = schema::all();
    let mut bundle = serde_json::Map::new();
    for (name, s) in &all {
        bundle.insert(name.to_string(), s.as_value().clone());
    }
    let Some(folder) = folder else {
        let json = Value::Object(bundle);
        let text = serde_json::to_string_pretty(&json).expect("json");
        return Ok(Out { json, text });
    };
    let save = |path: PathBuf, text: String| {
        std::fs::write(&path, text)
            .map_err(|e| VmmError::new(ErrorCode::SaveFailed, format!("Couldn't write {}: {e}", path.display())))
    };
    std::fs::create_dir_all(folder)
        .map_err(|e| VmmError::new(ErrorCode::SaveFailed, format!("Couldn't create {}: {e}", folder.display())))?;
    let mut written = Vec::new();
    for (name, s) in &all {
        let path = folder.join(format!("{name}.schema.json"));
        save(path.clone(), serde_json::to_string_pretty(s.as_value()).expect("json") + "\n")?;
        written.push(path.display().to_string());
    }
    let errors = folder.join("errors.json");
    save(errors.clone(), serde_json::to_string_pretty(&ErrorCodeList::current()).expect("json") + "\n")?;
    written.push(errors.display().to_string());
    let text = written.iter().map(|p| format!("Wrote {p}\n")).collect();
    Ok(Out { json: serde_json::json!({ "schemaVersion": SCHEMA_VERSION, "written": written }), text })
}

fn setups_text(l: &schema::SetupList) -> String {
    let mut t = String::new();
    if l.setups.is_empty() {
        t += "No saved setups.\n";
    }
    for s in &l.setups {
        let mark = if s.in_use { " (in use)" } else { "" };
        t += &format!("{}{mark}: {}\n", s.name, if s.order.is_empty() { "no mods".into() } else { s.order.join(", ") });
    }
    match &l.pass {
        None => t += "No compatibility pass under way.\n",
        Some(p) => {
            t += &format!(
                "Compatibility pass. Baseline: {}\n",
                if p.baseline.is_empty() { "none".into() } else { p.baseline.join(", ") }
            );
            for b in &p.batches {
                let state = match b.state {
                    schema::BatchState::Testing => "being tried",
                    schema::BatchState::Good => "good",
                    schema::BatchState::Bad => "bad",
                };
                let notes = if b.notes.is_empty() { String::new() } else { format!(" ({})", b.notes) };
                t += &format!("  {}: {} - {state}{notes}\n", b.name, b.mods.join(", "));
            }
        }
    }
    t
}

fn log_line(e: &schema::LogEvent) -> String {
    let level = match e.level {
        schema::LogLevel::Info => "info ",
        schema::LogLevel::Warn => "warn ",
        schema::LogLevel::Error => "ERROR",
    };
    format!("{}  {level}  {:<20} {:<10} {}\n", vmm_engine::events::timestamp(e.ts_ms), e.event, e.caller, e.message)
}

fn report_text(r: &schema::DeployReport) -> String {
    let mut t = match r.result {
        schema::DeployResult::Succeeded => "The last deploy succeeded".to_string(),
        schema::DeployResult::Failed => format!("The last deploy failed (exit code {})", r.exit_code.unwrap_or(-1)),
        schema::DeployResult::DidNotRun => "The last deploy didn't run".to_string(),
    };
    if let Some(ms) = r.duration_ms {
        t += &format!(" after {}m {}s", ms / 60_000, ms / 1000 % 60);
    }
    if let Some(at) = r.finished_at_ms {
        t += &format!(", at {} UTC", vmm_engine::events::timestamp(at));
    }
    t += ".\n";
    let mods = r.stages.iter().filter(|s| s.mod_id.is_some()).count();
    t += &format!("{} stages, {mods} of them deploying a mod.\n", r.stages.len());
    for e in &r.errors {
        t += &format!("Error: {e}\n");
    }
    for w in &r.warnings {
        let m = w.mod_name.as_deref().map(|m| format!("{m}: ")).unwrap_or_default();
        t += &format!("Warning ({}), {m}{} time(s). {}\n", w.code, w.count, w.detail);
    }
    if let Some(log) = &r.log {
        t += &format!("Log: {log} ({} lines)\n", r.log_lines);
    }
    t
}

fn status_text(s: &schema::StatusReport) -> String {
    let mut t = String::new();
    let Some(game) = &s.game else {
        return "No install of HITMAN World of Assassination was found.\n".into();
    };
    t += &format!("{} ({}): {}\n", game.name, game.store, game.folder);
    if let Some(b) = &s.backend {
        let version = b.version.clone().unwrap_or_else(|| "unknown version".into());
        match b.state {
            schema::BackendState::Installed => t += &format!("{} {version}\n", b.name),
            schema::BackendState::NotInstalled => t += &format!("{} isn't installed.\n", b.name),
            schema::BackendState::Unreadable => {
                t += &format!("{} can't be read: {}\n", b.name, b.problem.clone().unwrap_or_default())
            }
        }
    }
    t += &format!("Mods: {} installed, {} on", s.installed, s.enabled);
    if s.with_problems > 0 {
        t += &format!(", {} can't be read", s.with_problems);
    }
    t += "\n";
    if let Some(d) = &s.deploy {
        t += match (d.up_to_date, d.last_success) {
            (Some(true), _) => "The game has the current mods.\n",
            (_, None) => "Not deployed yet.\n",
            (_, Some(false)) => "The last deploy failed.\n",
            _ => "Changed since the last deploy: deploy for the game to use the changes.\n",
        };
        if let Some(busy) = &d.busy {
            t += &format!("{busy}\n");
        }
    }
    if !s.setups.is_empty() {
        t += &format!("Saved setups: {}\n", s.setups.join(", "));
    }
    if s.pass_active {
        t += "A compatibility pass is under way.\n";
    }
    for n in &s.notes {
        t += &format!("Note: {n}\n");
    }
    t
}

fn mod_line(m: &schema::ModView) -> String {
    let place = match m.position {
        Some(p) => format!("on {p:>3}"),
        None => "off    ".into(),
    };
    let problem = if m.problem.is_some() { "  (can't be read)" } else { "" };
    format!("{place}  {:<32} {}{problem}\n", m.id, m.name)
}

fn mod_text(m: &schema::ModView) -> String {
    let mut t = format!("{} ({})\n", m.name, m.id);
    if let Some(v) = &m.version {
        t += &format!("Version: {v}\n");
    }
    if !m.authors.is_empty() {
        t += &format!("By: {}\n", m.authors.join(", "));
    }
    t += &match m.position {
        Some(p) => format!("On, {p} in the load order\n"),
        None => "Off\n".into(),
    };
    let source = if m.kind_source == schema::KindSource::User { "chosen" } else { "from its files" };
    t += &format!("Type: {} ({source})\n", m.kind.label());
    if let Some(p) = &m.problem {
        t += &format!("Problem: {p}\n");
    }
    for o in &m.options {
        let mark = if o.chosen { "[x]" } else { "[ ]" };
        let group = o.group.as_ref().map(|g| format!(" (one of {g})")).unwrap_or_default();
        let needs = if o.needs.is_empty() { String::new() } else { format!(" needs {}", o.needs.join(", ")) };
        t += &format!("  {mark} {}{group}{needs}  value: {}\n", o.label, o.value);
    }
    if !m.load_before.is_empty() {
        t += &format!("Loads before: {}\n", m.load_before.join(", "));
    }
    if !m.load_after.is_empty() {
        t += &format!("Loads after: {}\n", m.load_after.join(", "));
    }
    if !m.replaced_by.is_empty() {
        t += &format!("Some of its files are replaced by: {}\n", m.replaced_by.join(", "));
    }
    t += &format!("Folder: {}\n", m.folder);
    t
}

fn order_text(r: &schema::OrderReport) -> String {
    let mut t = String::new();
    for (i, id) in r.order.iter().enumerate() {
        t += &format!("{:>3}. {id}\n", i + 1);
    }
    if r.order.is_empty() {
        t += "No mods are on.\n";
    }
    for b in &r.broken_rules {
        let rel = if b.relation == schema::RelationView::Before { "before" } else { "after" };
        t += &format!("Broken rule: {} should load {rel} {}\n", b.mod_id, b.other);
    }
    match &r.suggested {
        Some(s) if *s != r.order => t += &format!("Suggested order: {}\n", s.join(", ")),
        None => t += &format!("These mods' rules contradict each other: {}\n", r.contradictory.join(", ")),
        _ => {}
    }
    t
}
