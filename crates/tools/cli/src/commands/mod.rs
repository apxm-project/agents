//! Command modules for APXM CLI.

use std::fmt;

pub mod cli;
pub mod implementations;

pub mod agent;
pub(crate) mod dekk_hints;
pub mod interaction;
pub mod org;
pub mod process;
pub mod system;

pub use cli::*;

pub use agent::*;
pub use org::*;
pub use process::*;
pub use system::*;

#[derive(Debug)]
pub struct OutputAlreadyEmitted;

impl fmt::Display for OutputAlreadyEmitted {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("command output already emitted")
    }
}

impl std::error::Error for OutputAlreadyEmitted {}

#[allow(dead_code)]
pub(crate) fn output_already_emitted() -> anyhow::Error {
    anyhow::Error::new(OutputAlreadyEmitted)
}
