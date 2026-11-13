//! One error type for every backend and front end. Each error has a stable
//! `code` that tools and AI clients can rely on (ADR 0003, section 4), and a
//! plain-language `message` for people, which the window shows as it is.

use std::fmt;

/// Stable error codes. The strings from [`ErrorCode::as_str`] are a public
/// contract: once released, a code keeps its meaning and is never reused.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ErrorCode {
    /// No install of the game was found, or it moved.
    GameNotFound,
    /// The mod framework is not in the game folder.
    FrameworkMissing,
    /// The framework's settings file can't be read or isn't the expected shape.
    ConfigUnreadable,
    /// Another program changed the settings while a change was being saved.
    ConfigChangedMeanwhile,
    /// There is nowhere to keep a backup, or the backup failed.
    BackupUnavailable,
    /// A file could not be written.
    SaveFailed,
    /// No installed mod has that id.
    ModNotFound,
    /// The mod's own description can't be read.
    ModUnreadable,
    /// An option that the mod doesn't have, or two choices in one group.
    OptionInvalid,
    /// An option whose required mods are off.
    OptionRequirementOff,
    /// A new load order that doesn't hold exactly the mods that are on.
    OrderMismatch,
    /// The framework's deploy could not be started.
    DeployStartFailed,
    /// The game is running, so nothing that changes it can run.
    GameRunning,
    /// A deploy is already running.
    DeployRunning,
    /// The app's own settings, setups or records can't be read.
    SettingsUnreadable,
    /// Starting the game this way isn't supported for this copy.
    LaunchUnsupported,
    /// A compatibility pass step that doesn't fit its current state.
    PassState,
    /// Something else went wrong reading or writing a file.
    Io,
    /// A command line that `vmm` can't read, or a change without `--yes`.
    Usage,
    /// No plan has that id, or it was already applied.
    PlanNotFound,
    /// The plan is older than 10 minutes.
    PlanExpired,
    /// An AI tool tried to apply a change while Settings' "Let AI tools make
    /// changes" is off.
    AiChangesOff,
    /// The framework's deploy ran and reported failure.
    DeployFailed,
    /// Steam is running, so its settings can't be changed.
    SteamRunning,
    /// Steam, a signed-in account, or its settings can't be found or read.
    SteamUnavailable,
    /// A network request failed or got an answer that can't be read.
    NetworkFailed,
    /// A download didn't match the checksum its source gave, or isn't a
    /// readable archive.
    DownloadMismatch,
    /// The framework's update can't be applied automatically.
    SmfUpdateHeld,
}

impl ErrorCode {
    pub const ALL: [ErrorCode; 28] = [
        ErrorCode::GameNotFound,
        ErrorCode::FrameworkMissing,
        ErrorCode::ConfigUnreadable,
        ErrorCode::ConfigChangedMeanwhile,
        ErrorCode::BackupUnavailable,
        ErrorCode::SaveFailed,
        ErrorCode::ModNotFound,
        ErrorCode::ModUnreadable,
        ErrorCode::OptionInvalid,
        ErrorCode::OptionRequirementOff,
        ErrorCode::OrderMismatch,
        ErrorCode::DeployStartFailed,
        ErrorCode::GameRunning,
        ErrorCode::DeployRunning,
        ErrorCode::SettingsUnreadable,
        ErrorCode::LaunchUnsupported,
        ErrorCode::PassState,
        ErrorCode::Io,
        ErrorCode::Usage,
        ErrorCode::PlanNotFound,
        ErrorCode::PlanExpired,
        ErrorCode::AiChangesOff,
        ErrorCode::DeployFailed,
        ErrorCode::SteamRunning,
        ErrorCode::SteamUnavailable,
        ErrorCode::NetworkFailed,
        ErrorCode::DownloadMismatch,
        ErrorCode::SmfUpdateHeld,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            ErrorCode::GameNotFound => "game.not_found",
            ErrorCode::FrameworkMissing => "framework.missing",
            ErrorCode::ConfigUnreadable => "config.unreadable",
            ErrorCode::ConfigChangedMeanwhile => "config.changed_meanwhile",
            ErrorCode::BackupUnavailable => "backup.unavailable",
            ErrorCode::SaveFailed => "save.failed",
            ErrorCode::ModNotFound => "mod.not_found",
            ErrorCode::ModUnreadable => "mod.unreadable",
            ErrorCode::OptionInvalid => "option.invalid",
            ErrorCode::OptionRequirementOff => "option.requirement_off",
            ErrorCode::OrderMismatch => "order.mismatch",
            ErrorCode::DeployStartFailed => "deploy.start_failed",
            ErrorCode::GameRunning => "game.running",
            ErrorCode::DeployRunning => "deploy.running",
            ErrorCode::SettingsUnreadable => "settings.unreadable",
            ErrorCode::LaunchUnsupported => "launch.unsupported",
            ErrorCode::PassState => "pass.state",
            ErrorCode::Io => "io",
            ErrorCode::Usage => "usage.invalid",
            ErrorCode::PlanNotFound => "plan.not_found",
            ErrorCode::PlanExpired => "plan.expired",
            ErrorCode::AiChangesOff => "ai.changes_off",
            ErrorCode::DeployFailed => "deploy.failed",
            ErrorCode::SteamRunning => "steam.running",
            ErrorCode::SteamUnavailable => "steam.unavailable",
            ErrorCode::NetworkFailed => "network.failed",
            ErrorCode::DownloadMismatch => "download.mismatch",
            ErrorCode::SmfUpdateHeld => "smf.update_held",
        }
    }

    /// What the code means, for the list of codes.
    pub fn meaning(self) -> &'static str {
        match self {
            ErrorCode::GameNotFound => "No install of the game was found, or it has moved.",
            ErrorCode::FrameworkMissing => "The mod framework is not in the game folder.",
            ErrorCode::ConfigUnreadable => "The framework's settings file can't be read or isn't the expected shape.",
            ErrorCode::ConfigChangedMeanwhile => "Another program changed a file the change depends on.",
            ErrorCode::BackupUnavailable => "There is nowhere to keep a backup, or the backup failed.",
            ErrorCode::SaveFailed => "A file could not be written.",
            ErrorCode::ModNotFound => "No installed mod has that id.",
            ErrorCode::ModUnreadable => "The mod's own description can't be read.",
            ErrorCode::OptionInvalid => "An option or setup that doesn't exist, or two choices in one group.",
            ErrorCode::OptionRequirementOff => "An option whose required mods are off.",
            ErrorCode::OrderMismatch => "A load order that doesn't hold exactly the mods that are on.",
            ErrorCode::DeployStartFailed => "The framework's deploy could not be started.",
            ErrorCode::GameRunning => "The game is running.",
            ErrorCode::DeployRunning => "A deploy is running.",
            ErrorCode::SettingsUnreadable => "The app's own settings, setups or records can't be read.",
            ErrorCode::LaunchUnsupported => "Starting the game this way isn't supported for this copy.",
            ErrorCode::PassState => "A compatibility pass step that doesn't fit the pass's current state.",
            ErrorCode::Io => "Something else went wrong reading or writing a file.",
            ErrorCode::Usage => "A command line that vmm can't read, or a change without --yes.",
            ErrorCode::PlanNotFound => "No plan has that id, or it was already applied.",
            ErrorCode::PlanExpired => "The plan is more than 10 minutes old.",
            ErrorCode::AiChangesOff => "AI tools may read and plan, but changes from them are turned off in Settings.",
            ErrorCode::DeployFailed => "The framework's deploy ran and reported failure.",
            ErrorCode::SteamRunning => "Steam is running, so its settings can't be changed.",
            ErrorCode::SteamUnavailable => "Steam, a signed-in Steam account, or its settings can't be found or read.",
            ErrorCode::NetworkFailed => "A network request failed, or its answer can't be read.",
            ErrorCode::DownloadMismatch => {
                "A download doesn't match the checksum its source gave, or isn't a readable archive."
            }
            ErrorCode::SmfUpdateHeld => "SMF's update can't be applied automatically.",
        }
    }

    /// What to do about it, in a sentence.
    pub fn hint(self) -> &'static str {
        match self {
            ErrorCode::GameNotFound => "Install the game, or name its folder with --game (or VAEXIL_MOD_MANAGER_GAME).",
            ErrorCode::FrameworkMissing => "Install Simple Mod Framework into the game folder, then try again.",
            ErrorCode::ConfigUnreadable => {
                "Fix or restore the framework's config.json (VMM keeps backups), or open the framework once to rewrite it."
            }
            ErrorCode::ConfigChangedMeanwhile => "Read the state again, plan the change again, and apply the new plan.",
            ErrorCode::BackupUnavailable => "Make sure %LOCALAPPDATA% exists and can be written to.",
            ErrorCode::SaveFailed => "Check that the file isn't read-only or open in another program, then try again.",
            ErrorCode::ModNotFound => "List the mods and use an id from that list.",
            ErrorCode::ModUnreadable => "Reinstall the mod or ask its author; it can't be turned on until it reads.",
            ErrorCode::OptionInvalid => "Use the option values the mod lists, at most one per group.",
            ErrorCode::OptionRequirementOff => {
                "Turn on the mods the option needs first, in the same plan or before it."
            }
            ErrorCode::OrderMismatch => "Give every mod that is on exactly once, and no others.",
            ErrorCode::DeployStartFailed => {
                "Check that the framework's Deploy.exe exists and isn't blocked, then try again."
            }
            ErrorCode::GameRunning => "Close the game, then try again.",
            ErrorCode::DeployRunning => "Wait for the deploy to finish, then try again.",
            ErrorCode::SettingsUnreadable => {
                "Fix or move aside the named file in %LOCALAPPDATA%\\Vaexil Mod Manager; VMM won't overwrite it."
            }
            ErrorCode::LaunchUnsupported => "Start the game from its own launcher.",
            ErrorCode::PassState => "Read the pass's state and take the step that fits it.",
            ErrorCode::Io => "Check the named file and folder, then try again.",
            ErrorCode::Usage => "Run `vmm help` for the commands. Changes need --yes.",
            ErrorCode::PlanNotFound => "Plan the change again and apply the plan id it gives.",
            ErrorCode::PlanExpired => "Plan the change again and apply the new plan.",
            ErrorCode::AiChangesOff => {
                "Tell the user. They can turn on \"Let AI tools make changes\" in Vaexil Mod Manager's Settings, or make the change themselves."
            }
            ErrorCode::DeployFailed => {
                "Read the deploy report (vmm deploy-report): its errors and the log name the mod or file that failed."
            }
            ErrorCode::SteamRunning => "Exit Steam completely (Steam menu > Exit), then try again.",
            ErrorCode::SteamUnavailable => {
                "Check that Steam is installed and you have signed in on this PC at least once."
            }
            ErrorCode::NetworkFailed => "Check the internet connection, then try again.",
            ErrorCode::DownloadMismatch => {
                "Try again; if it keeps happening, the release may have changed. Nothing was installed."
            }
            ErrorCode::SmfUpdateHeld => {
                "Read SMF's release notes and update it by hand, as SMF's own Mod Manager would ask you to."
            }
        }
    }

    /// The exit code `vmm` uses: 2 bad usage, 3 refused by a rule, 4 changed
    /// meanwhile, 5 the game or a deploy is running, 6 the deploy ran and
    /// failed, 10 unexpected.
    pub fn exit_code(self) -> i32 {
        match self {
            ErrorCode::Usage => 2,
            ErrorCode::ConfigChangedMeanwhile => 4,
            ErrorCode::GameRunning | ErrorCode::DeployRunning | ErrorCode::SteamRunning => 5,
            ErrorCode::DeployFailed => 6,
            ErrorCode::SaveFailed | ErrorCode::Io | ErrorCode::NetworkFailed => 10,
            _ => 3,
        }
    }

    /// Whether trying the same thing again later can work without changing
    /// anything first.
    pub fn retryable(self) -> bool {
        matches!(
            self,
            ErrorCode::ConfigChangedMeanwhile
                | ErrorCode::GameRunning
                | ErrorCode::DeployRunning
                | ErrorCode::SteamRunning
                | ErrorCode::NetworkFailed
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VmmError {
    pub code: ErrorCode,
    /// What went wrong and what was (not) changed, for people.
    pub message: String,
}

impl VmmError {
    pub fn new(code: ErrorCode, message: impl Into<String>) -> VmmError {
        VmmError { code, message: message.into() }
    }
}

impl fmt::Display for VmmError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for VmmError {}

/// `Result` with [`VmmError`].
pub type Result<T> = std::result::Result<T, VmmError>;

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;

    #[test]
    fn codes_are_unique_and_dotted() {
        let strings: HashSet<&str> = ErrorCode::ALL.iter().map(|c| c.as_str()).collect();
        assert_eq!(strings.len(), ErrorCode::ALL.len());
        assert!(strings.iter().all(|s| s.chars().all(|c| c.is_ascii_lowercase() || c == '.' || c == '_')));
        assert!(ErrorCode::ConfigChangedMeanwhile.retryable() && !ErrorCode::ModNotFound.retryable());
        assert_eq!(VmmError::new(ErrorCode::Io, "disk full").to_string(), "disk full");
    }
}
