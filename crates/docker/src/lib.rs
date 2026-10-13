//! Engine-facing domain model.
//!
//! These are Berth's own types — never bollard's — so the UI can run against
//! fixtures (tests, screenshots, design work) exactly as it runs against a
//! live engine. See docs/plan/03-architecture.md.

pub mod discovery;
mod engine;
mod files;
pub mod fixtures;
#[cfg(feature = "live")]
pub mod live;
mod model;
mod tree;

pub use discovery::*;
pub use engine::*;
pub use files::*;
#[cfg(feature = "live")]
pub use live::BollardEngine;
pub use model::*;
pub use tree::*;
