//! The shared contract for Vaexil Mod Manager: the [`backend::Backend`] trait
//! and the types it reports, [`error::VmmError`] with its stable codes, game
//! discovery, load-order reasoning that no backend owns, and [`schema`], the
//! versioned JSON that the command line and AI tools see.
//!
//! Nothing in this crate writes. Backends (such as `vmm-backend-smf`) write
//! their framework's own settings; the engine (`vmm-engine`) writes the app's
//! own files. `docs/safety-model.md` lists every file that is read or written.

pub mod backend;
pub mod cmdline;
pub mod error;
pub mod game;
pub mod order;
pub mod paths;
pub mod schema;
pub mod steam;
