//! Connecting AI apps (Claude Code, Gemini CLI, Codex CLI) to `vmm mcp`.
//!
//! The app never writes another app's settings itself (ADR 0003): Connect
//! and Disconnect run that app's own `mcp add` and `mcp remove`, after
//! Settings has shown the exact command. Each app is looked for where it is
//! really installed, not only on PATH: on JASON-PC, Claude Code exists only
//! inside the Claude desktop app, so a pasted `claude mcp add` failed.
//!
//! Whether a connection works is shown two ways, neither costing AI usage:
//! the app's own status command (which starts the server and handshakes), and
//! the record `vmm mcp` keeps of each AI app that connects to it.
//!
//! This mirrors jamarq-agent-kit's `connect` module (kit v4); a kit fix goes
//! to the kit first.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use vmm_core::error::{ErrorCode, VmmError};
use vmm_core::schema::LogLevel;

use crate::Engine;

/// The server's name in the AI apps' settings. Letters, digits and hyphens.
pub const SERVER_NAME: &str = "vaexil-mod-manager";

/// The event `vmm mcp` logs when an AI app connects.
pub const CLIENT_SEEN: &str = "mcp.client_seen";

/// How long an AI app's own command may take.
const COMMAND_TIMEOUT: Duration = Duration::from_secs(60);
/// A test request runs the AI once, which takes longer.
const TEST_TIMEOUT: Duration = Duration::from_secs(180);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AiApp {
    ClaudeCode,
    GeminiCli,
    CodexCli,
}

impl AiApp {
    pub const ALL: [AiApp; 3] = [AiApp::ClaudeCode, AiApp::GeminiCli, AiApp::CodexCli];

    pub fn name(self) -> &'static str {
        match self {
            AiApp::ClaudeCode => "Claude Code",
            AiApp::GeminiCli => "Gemini CLI",
            AiApp::CodexCli => "Codex CLI",
        }
    }

    /// Its command's name, without `.exe` or `.cmd`.
    fn command(self) -> &'static str {
        match self {
            AiApp::ClaudeCode => "claude",
            AiApp::GeminiCli => "gemini",
            AiApp::CodexCli => "codex",
        }
    }

    /// Where to get it.
    pub fn install_page(self) -> &'static str {
        match self {
            AiApp::ClaudeCode => "https://claude.com/claude-code",
            AiApp::GeminiCli => "https://github.com/google-gemini/gemini-cli",
            AiApp::CodexCli => "https://github.com/openai/codex",
        }
    }

    /// Whether `client` (the name an MCP client gave) is this app.
    pub fn is_client(self, client: &str) -> bool {
        client.to_lowercase().contains(self.command())
    }
}

/// Where an AI app was found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Found {
    pub path: PathBuf,
    /// The copy inside the Claude desktop app. It works for `mcp add`, but
    /// isn't signed in on its own, so it can't run a request headless.
    pub desktop_copy: bool,
}

/// The folders to look in, from the environment (tests pass their own).
#[derive(Debug, Clone, Default)]
pub struct Places {
    pub path: Vec<PathBuf>,
    pub home: Option<PathBuf>,
    pub appdata: Option<PathBuf>,
    pub local_appdata: Option<PathBuf>,
}

impl Places {
    pub fn from_env() -> Places {
        let var = |k: &str| std::env::var_os(k).filter(|v| !v.is_empty()).map(PathBuf::from);
        Places {
            path: std::env::var_os("PATH").map(|p| std::env::split_paths(&p).collect()).unwrap_or_default(),
            home: var("USERPROFILE"),
            appdata: var("APPDATA"),
            local_appdata: var("LOCALAPPDATA"),
        }
    }
}

/// Looks for `app`: on PATH, where its installers put it, and for Claude
/// Code, the newest copy inside the Claude desktop app.
pub fn find_in(app: AiApp, places: &Places) -> Option<Found> {
    let cmd = app.command();
    let plain = |path: PathBuf| path.is_file().then_some(Found { path, desktop_copy: false });
    let on_path =
        places.path.iter().flat_map(|d| [d.join(format!("{cmd}.exe")), d.join(format!("{cmd}.cmd"))]).find_map(plain);
    if on_path.is_some() {
        return on_path;
    }
    if app == AiApp::ClaudeCode
        && let Some(f) = places.home.as_ref().and_then(|h| plain(h.join(".local").join("bin").join("claude.exe")))
    {
        return Some(f);
    }
    if let Some(f) = places.appdata.as_ref().and_then(|a| plain(a.join("npm").join(format!("{cmd}.cmd")))) {
        return Some(f);
    }
    if app == AiApp::ClaudeCode {
        return places.local_appdata.as_deref().and_then(desktop_claude);
    }
    None
}

pub fn find(app: AiApp) -> Option<Found> {
    find_in(app, &Places::from_env())
}

/// The newest `Claude-Data\<id>\claude-code\<version>\<hash>\claude.exe`.
fn desktop_claude(local_appdata: &Path) -> Option<Found> {
    let dirs = |p: &Path| -> Vec<PathBuf> {
        std::fs::read_dir(p).map(|r| r.flatten().map(|e| e.path()).filter(|p| p.is_dir()).collect()).unwrap_or_default()
    };
    // A version folder such as `2.1.325`, compared part by part.
    let version = |name: &str| -> Option<Vec<u64>> { name.split('.').map(|p| p.parse().ok()).collect() };
    let mut best: Option<(Vec<u64>, PathBuf)> = None;
    for profile in dirs(&local_appdata.join("Claude-Data")) {
        for version_dir in dirs(&profile.join("claude-code")) {
            let Some(version) = version_dir.file_name().and_then(|n| version(&n.to_string_lossy())) else {
                continue;
            };
            for hash in dirs(&version_dir) {
                let exe = hash.join("claude.exe");
                if exe.is_file() && best.as_ref().is_none_or(|(v, _)| version > *v) {
                    best = Some((version.clone(), exe));
                }
            }
        }
    }
    best.map(|(_, path)| Found { path, desktop_copy: true })
}

/// A command to run, and to show exactly before it runs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandLine {
    pub program: PathBuf,
    pub args: Vec<String>,
}

impl CommandLine {
    /// As it would be typed in a terminal.
    pub fn display(&self) -> String {
        let quote = |s: &str| {
            if s.is_empty() || s.contains([' ', '\t', '"']) {
                format!("\"{}\"", s.replace('"', "\\\""))
            } else {
                s.to_string()
            }
        };
        std::iter::once(quote(&self.program.to_string_lossy()))
            .chain(self.args.iter().map(|a| quote(a)))
            .collect::<Vec<_>>()
            .join(" ")
    }

    /// Runs it without a window and returns its output, stopping it after
    /// `timeout`. Rust's standard library quotes arguments for a `.cmd`
    /// shim, or refuses ones it can't pass safely.
    fn run(&self, timeout: Duration) -> Result<Output, String> {
        let mut cmd = std::process::Command::new(&self.program);
        cmd.args(&self.args)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            cmd.creation_flags(CREATE_NO_WINDOW);
        }
        let mut child = cmd.spawn().map_err(|e| format!("couldn't start {}: {e}", self.program.display()))?;
        let (mut out, mut err) = (child.stdout.take(), child.stderr.take());
        let read = |s: Option<&mut dyn std::io::Read>| {
            let mut buf = Vec::new();
            if let Some(s) = s {
                let _ = s.read_to_end(&mut buf);
            }
            String::from_utf8_lossy(&buf).into_owned()
        };
        let out_thread = std::thread::spawn(move || read(out.as_mut().map(|s| s as &mut dyn std::io::Read)));
        let err_thread = std::thread::spawn(move || read(err.as_mut().map(|s| s as &mut dyn std::io::Read)));
        let start = Instant::now();
        let status = loop {
            if let Some(s) = child.try_wait().map_err(|e| e.to_string())? {
                break s;
            }
            if start.elapsed() > timeout {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!("it didn't finish within {} seconds", timeout.as_secs()));
            }
            std::thread::sleep(Duration::from_millis(100));
        };
        Ok(Output {
            success: status.success(),
            stdout: out_thread.join().unwrap_or_default(),
            stderr: err_thread.join().unwrap_or_default(),
        })
    }
}

#[derive(Debug, Clone)]
struct Output {
    success: bool,
    stdout: String,
    stderr: String,
}

impl Output {
    /// Both streams, trimmed, for showing.
    fn text(&self) -> String {
        [self.stdout.trim(), self.stderr.trim()].into_iter().filter(|s| !s.is_empty()).collect::<Vec<_>>().join("\n")
    }
}

/// `vmm mcp`, as the AI app should start it.
fn server_args(vmm_exe: &Path) -> Vec<String> {
    vec![vmm_exe.to_string_lossy().into_owned(), "mcp".into()]
}

/// The app's own command that adds `vmm mcp`. Claude Code and Gemini CLI
/// get user scope (every project), not their default of the current one.
pub fn connect_command(app: AiApp, found: &Found, vmm_exe: &Path) -> CommandLine {
    let mut args: Vec<String> = match app {
        AiApp::ClaudeCode => vec!["mcp", "add", "--scope", "user", SERVER_NAME, "--"],
        AiApp::GeminiCli => vec!["mcp", "add", "--scope", "user", SERVER_NAME],
        AiApp::CodexCli => vec!["mcp", "add", SERVER_NAME, "--"],
    }
    .into_iter()
    .map(String::from)
    .collect();
    args.extend(server_args(vmm_exe));
    CommandLine { program: found.path.clone(), args }
}

/// The app's own command that removes it again.
pub fn disconnect_command(app: AiApp, found: &Found) -> CommandLine {
    let args: &[&str] = match app {
        AiApp::ClaudeCode | AiApp::GeminiCli => &["mcp", "remove", "--scope", "user", SERVER_NAME],
        AiApp::CodexCli => &["mcp", "remove", SERVER_NAME],
    };
    CommandLine { program: found.path.clone(), args: args.iter().map(|s| s.to_string()).collect() }
}

/// The app's own status command. Claude Code's and Gemini CLI's start the
/// server and handshake with it; Codex CLI's lists what is configured.
pub fn check_command(app: AiApp, found: &Found) -> CommandLine {
    let args: &[&str] = match app {
        AiApp::ClaudeCode => &["mcp", "get", SERVER_NAME],
        AiApp::GeminiCli | AiApp::CodexCli => &["mcp", "list"],
    };
    CommandLine { program: found.path.clone(), args: args.iter().map(|s| s.to_string()).collect() }
}

/// The request the test sends: read-only, one sentence back.
pub const TEST_REQUEST: &str =
    "Use vaexil-mod-manager to tell me my SMF version and how many mods are on, in one sentence.";

/// A headless test that lets the AI use only the status tool. Only Claude
/// Code can be limited to one tool, and only a copy signed in outside the
/// desktop app can run headless; elsewhere the user sends the request.
pub fn test_command(app: AiApp, found: &Found) -> Option<CommandLine> {
    (app == AiApp::ClaudeCode && !found.desktop_copy).then(|| CommandLine {
        program: found.path.clone(),
        args: vec![
            "-p".into(),
            TEST_REQUEST.into(),
            "--allowedTools".into(),
            format!("mcp__{SERVER_NAME}__get_status"),
        ],
    })
}

/// What the status command said about the server: connected, failed, or
/// not configured. `None` when it can't tell (Codex lists without checking).
fn read_check(app: AiApp, out: &Output) -> Check {
    let text = out.text();
    let line = text.lines().find(|l| l.contains(SERVER_NAME) || l.contains("Status:")).unwrap_or("");
    let lower = text.to_lowercase();
    let listed = text.contains(SERVER_NAME);
    match app {
        // "No MCP server named "x". Configured servers: ..." (Claude Code 2.1.325).
        AiApp::ClaudeCode if lower.contains("no mcp server") => Check::NotConnected,
        AiApp::ClaudeCode if !out.success => Check::Failed(text),
        AiApp::ClaudeCode if lower.contains("connected") && !lower.contains("failed") => Check::Connected,
        AiApp::ClaudeCode => Check::Failed(text),
        _ if !listed => Check::NotConnected,
        AiApp::GeminiCli if line.to_lowercase().contains("disconnected") => Check::Failed(line.trim().to_string()),
        AiApp::GeminiCli if line.to_lowercase().contains("connected") => Check::Connected,
        _ => Check::Listed,
    }
}

/// The outcome of [`Engine::check_ai_app`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Check {
    /// The app started the server and it answered.
    Connected,
    /// The app has it in its settings but doesn't check it (Codex CLI).
    Listed,
    /// The app doesn't have it.
    NotConnected,
    /// The app has it, but starting it failed: the app's own words.
    Failed(String),
}

/// The last time an AI app connected to `vmm mcp`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LastClient {
    /// The name and version the app gave, such as `claude-code 2.1.325`.
    pub client: String,
    pub at_ms: u64,
}

fn unavailable(app: AiApp) -> VmmError {
    VmmError::new(ErrorCode::AiAppUnavailable, format!("{} wasn't found on this PC.", app.name()))
}

impl Engine {
    /// Records that an AI app connected to `vmm mcp` (the engine's caller).
    pub fn note_client(&self, name: &str, version: &str) {
        self.log(LogLevel::Info, CLIENT_SEEN, None, None, format!("{name} {version}").trim().to_string(), None);
    }

    /// The last time `app` connected to `vmm mcp`, from the event log.
    pub fn last_client(&self, app: AiApp) -> Option<LastClient> {
        let dir = self.logs_dir()?;
        crate::events::recent(&dir, 2000)
            .into_iter()
            .rev()
            .find(|e| e.event == CLIENT_SEEN && app.is_client(&e.caller))
            .map(|e| LastClient { client: e.message, at_ms: e.ts_ms })
    }

    /// Runs `app`'s own `mcp add` for `vmm mcp`.
    pub fn connect_ai_app(&self, app: AiApp, vmm_exe: &Path) -> Result<String, VmmError> {
        let r = find(app).ok_or_else(|| unavailable(app)).and_then(|f| {
            let cmd = connect_command(app, &f, vmm_exe);
            run_change(app, &cmd, "connect")
                .map(|()| format!("{} now starts Vaexil Mod Manager's tools. New sessions see them.", app.name()))
        });
        self.note_ai_app("ai_app.connected", app, &r);
        r
    }

    /// Runs `app`'s own `mcp remove`.
    pub fn disconnect_ai_app(&self, app: AiApp) -> Result<String, VmmError> {
        let r = find(app).ok_or_else(|| unavailable(app)).and_then(|f| {
            run_change(app, &disconnect_command(app, &f), "disconnect")
                .map(|()| format!("{} no longer has Vaexil Mod Manager's tools.", app.name()))
        });
        self.note_ai_app("ai_app.disconnected", app, &r);
        r
    }

    /// Runs `app`'s own status command and reads what it says.
    pub fn check_ai_app(&self, app: AiApp) -> Result<Check, VmmError> {
        let f = find(app).ok_or_else(|| unavailable(app))?;
        let out = check_command(app, &f)
            .run(COMMAND_TIMEOUT)
            .map_err(|e| VmmError::new(ErrorCode::Io, format!("Couldn't ask {}: {e}", app.name())))?;
        Ok(read_check(app, &out))
    }

    /// Sends the test request headless, where the app can run one limited to
    /// the status tool, and returns the AI's answer. This uses the user's AI
    /// plan or credits.
    pub fn test_ai_app(&self, app: AiApp) -> Result<String, VmmError> {
        let f = find(app).ok_or_else(|| unavailable(app))?;
        let cmd = test_command(app, &f).ok_or_else(|| {
            VmmError::new(
                ErrorCode::AiAppUnavailable,
                format!("{} can't be tested from here; send it the request instead.", app.name()),
            )
        })?;
        let out = cmd
            .run(TEST_TIMEOUT)
            .map_err(|e| VmmError::new(ErrorCode::Io, format!("Couldn't ask {}: {e}", app.name())))?;
        let text = out.text();
        if out.success && !text.is_empty() {
            Ok(text)
        } else {
            Err(VmmError::new(
                ErrorCode::Io,
                format!("{} didn't answer: {}", app.name(), if text.is_empty() { "no output" } else { &text }),
            ))
        }
    }

    fn note_ai_app(&self, event: &str, app: AiApp, r: &Result<String, VmmError>) {
        match r {
            Ok(m) => self.log(LogLevel::Info, event, None, None, m.clone(), None),
            Err(e) => self.log(LogLevel::Error, event, None, None, format!("{}: {}", app.name(), e.message), Some(e)),
        }
    }
}

fn run_change(app: AiApp, cmd: &CommandLine, what: &str) -> Result<(), VmmError> {
    let out = cmd
        .run(COMMAND_TIMEOUT)
        .map_err(|e| VmmError::new(ErrorCode::Io, format!("Couldn't run {}: {e}", app.name())))?;
    if out.success {
        Ok(())
    } else {
        Err(VmmError::new(ErrorCode::Io, format!("{} couldn't {what}: {}", app.name(), out.text())))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn touch(p: &Path) {
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, "").unwrap();
    }

    #[test]
    fn finds_apps_where_they_are_installed() {
        let tmp = tempfile::tempdir().unwrap();
        let t = tmp.path();
        let places = Places {
            path: vec![t.join("bin")],
            home: Some(t.join("home")),
            appdata: Some(t.join("roaming")),
            local_appdata: Some(t.join("local")),
        };
        assert_eq!(find_in(AiApp::ClaudeCode, &places), None);
        // Only inside the desktop app: the newest version wins.
        for v in ["2.1.321", "2.1.325", "notaversion"] {
            touch(&t.join(format!("local/Claude-Data/abc/claude-code/{v}/h1/claude.exe")));
        }
        let f = find_in(AiApp::ClaudeCode, &places).unwrap();
        assert!(f.desktop_copy && f.path.to_string_lossy().contains("2.1.325"), "{f:?}");
        assert!(test_command(AiApp::ClaudeCode, &f).is_none(), "the desktop copy can't run headless");
        // npm's shim comes before the desktop copy, PATH before both.
        touch(&t.join("roaming/npm/claude.cmd"));
        assert_eq!(find_in(AiApp::ClaudeCode, &places).unwrap().path, t.join("roaming/npm/claude.cmd"));
        touch(&t.join("bin/claude.exe"));
        let f = find_in(AiApp::ClaudeCode, &places).unwrap();
        assert_eq!((f.path.clone(), f.desktop_copy), (t.join("bin/claude.exe"), false));
        assert!(test_command(AiApp::ClaudeCode, &f).is_some());
        touch(&t.join("roaming/npm/gemini.cmd"));
        assert_eq!(find_in(AiApp::GeminiCli, &places).unwrap().path, t.join("roaming/npm/gemini.cmd"));
        assert_eq!(find_in(AiApp::CodexCli, &places), None);
    }

    #[test]
    fn commands_are_each_apps_own() {
        let f = Found { path: PathBuf::from(r"C:\x\claude.exe"), desktop_copy: false };
        let vmm = Path::new(r"C:\Program Files\VMM\vmm.exe");
        assert_eq!(
            connect_command(AiApp::ClaudeCode, &f, vmm).display(),
            r#"C:\x\claude.exe mcp add --scope user vaexil-mod-manager -- "C:\Program Files\VMM\vmm.exe" mcp"#
        );
        assert_eq!(
            connect_command(AiApp::GeminiCli, &f, vmm).args.join(" "),
            r"mcp add --scope user vaexil-mod-manager C:\Program Files\VMM\vmm.exe mcp"
        );
        assert_eq!(connect_command(AiApp::CodexCli, &f, vmm).args[..4], ["mcp", "add", "vaexil-mod-manager", "--"]);
        assert_eq!(disconnect_command(AiApp::CodexCli, &f).args, ["mcp", "remove", "vaexil-mod-manager"]);
        assert_eq!(check_command(AiApp::ClaudeCode, &f).args, ["mcp", "get", "vaexil-mod-manager"]);
        assert!(test_command(AiApp::GeminiCli, &f).is_none());
    }

    #[test]
    fn reads_what_each_status_command_says() {
        let out = |success: bool, stdout: &str| Output { success, stdout: stdout.into(), stderr: String::new() };
        let claude_ok = "vaexil-mod-manager:\n  Scope: User config\n  Status: \u{221a} Connected\n";
        assert_eq!(read_check(AiApp::ClaudeCode, &out(true, claude_ok)), Check::Connected);
        let claude_bad = "vaexil-mod-manager:\n  Status: \u{2717} Failed to connect\n";
        assert!(matches!(read_check(AiApp::ClaudeCode, &out(true, claude_bad)), Check::Failed(_)));
        let missing = "No MCP server named \"vaexil-mod-manager\". Configured servers: keeplight";
        assert_eq!(read_check(AiApp::ClaudeCode, &out(false, missing)), Check::NotConnected);
        let gemini = "Configured MCP servers:\n\n\u{2713} vaexil-mod-manager: vmm.exe mcp (stdio) - Connected\n";
        assert_eq!(read_check(AiApp::GeminiCli, &out(true, gemini)), Check::Connected);
        let gemini_bad = "\u{2717} vaexil-mod-manager: vmm.exe mcp (stdio) - Disconnected\n";
        assert!(matches!(read_check(AiApp::GeminiCli, &out(true, gemini_bad)), Check::Failed(_)));
        assert_eq!(
            read_check(AiApp::CodexCli, &out(true, "Name  Command\nvaexil-mod-manager  vmm.exe\n")),
            Check::Listed
        );
        assert_eq!(read_check(AiApp::CodexCli, &out(true, "No MCP servers configured yet.")), Check::NotConnected);
    }

    #[test]
    fn knows_each_apps_client_name() {
        assert!(AiApp::ClaudeCode.is_client("mcp:claude-code"));
        assert!(AiApp::GeminiCli.is_client("mcp:gemini-cli-mcp-client"));
        assert!(AiApp::CodexCli.is_client("mcp:codex-mcp-client"));
        assert!(!AiApp::CodexCli.is_client("mcp:claude-code"));
    }
}
