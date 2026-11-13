//! Platform-neutral logic for Vaexil Mod Manager.
//!
//! The crate writes one file: a framework's own settings, when a mod is turned
//! on or off or its options are saved ([`framework::Framework::set_enabled`],
//! [`framework::Framework::set_options`]). Everything else in game
//! folders and mod frameworks is only ever opened for reading. The safety
//! model in `docs/safety-model.md` lists every file that is read or written.

pub mod framework;
pub mod game;
pub mod paths;
pub mod smf;
pub mod steam;
