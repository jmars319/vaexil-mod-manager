//! Read-only Steam lookups: library folders and installed apps. Copied from
//! Modlist Launcher, which also owns the launch-options code that a later
//! stage brings over for Steam's Play button.

pub mod library;
pub mod vdf;

pub use library::{Account, AppInfo, SteamRoot};
