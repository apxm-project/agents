//! Command modules for APXM CLI.

use std::fmt;

pub mod cli;
pub mod implementations;

pub mod agent;
pub mod analysis;
#[cfg(feature = "driver")]
pub mod backend;
pub mod cache;
pub mod canonical_air;
pub mod canonical_execute;
#[cfg(feature = "driver")]
pub mod chat;
pub mod codegen;
#[cfg(feature = "driver")]
pub mod compile_service_canonical;
pub(crate) mod dekk_hints;
pub mod integration;
pub mod ops;
pub mod org;
pub mod process;
#[cfg(feature = "driver")]
pub mod render;
#[cfg(feature = "driver")]
pub mod rollout;
pub mod session;
#[cfg(feature = "driver")]
pub(crate) mod sse_permissions;
pub mod system;
pub mod team;
pub mod template;
pub mod tokenize;
pub mod tool;
#[cfg(feature = "driver")]
pub mod watch;

pub use cli::*;

pub use agent::*;
pub use analysis::*;
#[cfg(feature = "driver")]
pub use backend::*;
pub use cache::*;
pub use canonical_air::*;
pub use canonical_execute::*;
pub use codegen::*;
pub use integration::*;
pub use ops::*;
pub use org::*;
pub use process::*;
pub use session::*;
pub use system::*;
pub use team::*;
pub use template::*;
pub use tokenize::*;
pub use tool::*;
#[cfg(feature = "driver")]
pub use watch::*;

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
