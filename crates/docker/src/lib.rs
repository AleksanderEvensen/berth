//! Engine-facing domain model.
//!
//! These are Berth's own types — never bollard's — so the UI can run against
//! fixtures (tests, screenshots, design work) exactly as it runs against a
//! live engine. See docs/plan/03-architecture.md.

pub mod fixtures;
mod model;
mod tree;

pub use model::*;
pub use tree::*;
