//! Engine-facing domain model.
//!
//! These are Berth's own types — never bollard's — so the UI can run against
//! fixtures (tests, screenshots, design work) exactly as it runs against a
//! live engine.

pub mod discovery;
mod engine;
mod files;
pub mod fixtures;
#[cfg(feature = "live")]
pub mod live;
mod model;
// Only the live client spawns `ssh`; detection is shared with settings.
#[cfg_attr(not(feature = "live"), allow(dead_code))]
mod ssh;
mod tree;

pub use discovery::*;
pub use engine::*;
pub use files::*;
#[cfg(feature = "live")]
pub use live::BollardEngine;
pub use model::*;
pub use ssh::{SSH_CLIENT_MISSING, ssh_client};
pub use tree::*;
