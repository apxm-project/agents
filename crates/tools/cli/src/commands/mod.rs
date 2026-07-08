//! Command modules for APXM CLI.

use std::fmt;

pub mod cli;
pub mod implementations;

pub mod acp;
pub mod agent;
pub mod analysis;
#[cfg(feature = "driver")]
pub mod backend;
pub mod cache;
#[cfg(feature = "driver")]
pub mod chat;
pub mod codegen;
#[cfg(feature = "driver")]
pub mod compile;
pub(crate) mod dekk_hints;
#[cfg(feature = "driver")]
pub mod execute;
pub mod frontend_air;
pub mod integration;
pub mod ops;
pub mod org;
pub mod process;
#[cfg(feature = "driver")]
pub mod render;
pub mod replay;
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
pub mod workflow;

pub use cli::*;

pub use acp::*;
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
pub use frontend_air::*;
pub use integration::*;
pub use ops::*;
pub use org::*;
pub use process::*;
pub use replay::*;
pub use session::*;
pub use system::*;
pub use team::*;
pub use template::*;
pub use tokenize::*;
pub use tool::*;
#[cfg(feature = "driver")]
pub use watch::*;
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
