//! Platform-neutral logic for Vaexil Mod Manager.
//!
//! Nothing in this crate writes anywhere. Game folders and mod frameworks are
//! only ever opened for reading; the safety model in `docs/safety-model.md`
//! lists every file that is read.

pub mod framework;
pub mod game;
pub mod paths;
pub mod smf;
pub mod steam;
