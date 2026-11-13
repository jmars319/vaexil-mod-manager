//! Steam lookups (library folders, installed apps, accounts) and the
//! launch-options edit behind Steam's Play button. Copied from Modlist
//! Launcher, which owns the originals.

pub mod launch_options;
pub mod library;
pub mod vdf;

pub use launch_options::{
    LaunchOptionsError, RestorePlan, get_launch_options, is_ours, plan_restore, set_launch_options,
    wrapper_launch_options,
};
pub use library::{Account, AppInfo, SteamRoot};
