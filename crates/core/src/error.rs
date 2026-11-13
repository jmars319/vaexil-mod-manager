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
}

impl ErrorCode {
    pub const ALL: [ErrorCode; 18] = [
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
        }
    }

    /// Whether trying the same thing again later can work without changing
    /// anything first.
    pub fn retryable(self) -> bool {
        matches!(self, ErrorCode::ConfigChangedMeanwhile | ErrorCode::GameRunning | ErrorCode::DeployRunning)
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
