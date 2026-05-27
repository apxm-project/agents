//! Command modules for APXM CLI.

use std::fmt;

pub mod cli;
pub mod implementations;

pub mod agent;
pub mod analysis;
#[cfg(feature = "driver")]
pub mod backend;
pub mod cache;
pub mod codegen;
#[cfg(feature = "driver")]
pub mod compile;
pub(crate) mod dekk_hints;
#[cfg(feature = "driver")]
pub mod execute;
pub mod gui;
pub mod ops;
pub mod process;
pub mod replay;
#[cfg(feature = "driver")]
pub mod render;
#[cfg(feature = "driver")]
pub mod rollout;
pub mod session;
pub mod system;
#[cfg(feature = "driver")]
pub mod watch;
pub mod team;
pub mod template;
pub mod tokenize;
pub mod tool;
pub mod workflow;

// Re-export CLI types
pub use cli::*;

// Shared helpers in `implementations` are only consumed by tests in main.rs
// (sibling modules import them directly via `super::implementations::X`).
#[cfg(test)]
pub(crate) use implementations::*;

pub use agent::*;
pub use analysis::*;
#[cfg(feature = "driver")]
pub use backend::*;
pub use cache::*;
pub use codegen::*;
#[cfg(feature = "driver")]
pub use compile::*;
#[cfg(feature = "driver")]
pub use execute::*;
pub use gui::*;
pub use ops::*;
pub use process::*;
pub use replay::*;
// rollout::* is consumed via the `commands::rollout::...` path in main.rs;
// no glob-export needed here.
pub use session::*;
pub use system::*;
#[cfg(feature = "driver")]
pub use watch::*;
pub use team::*;
pub use template::*;
pub use tokenize::*;
pub use tool::*;
pub use workflow::*;

#[derive(Debug)]
pub struct OutputAlreadyEmitted;

impl fmt::Display for OutputAlreadyEmitted {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("command output already emitted")
    }
}

impl std::error::Error for OutputAlreadyEmitted {}

pub(crate) fn output_already_emitted() -> anyhow::Error {
    anyhow::Error::new(OutputAlreadyEmitted)
}
