//! Command modules for the Dekk-owned `apxm-dev` binary.

use std::fmt;

#[path = "../../../cli/src/commands/agent.rs"]
pub mod agent;
#[path = "../../../cli/src/commands/cli.rs"]
pub mod cli;
#[path = "../../../cli/src/commands/dekk_hints.rs"]
pub(crate) mod dekk_hints;
#[path = "../../../cli/src/commands/implementations.rs"]
pub mod implementations;
#[path = "../../../cli/src/commands/org.rs"]
pub mod org;

pub mod analysis;
pub mod canonical_air;
pub mod canonical_execute;
pub mod codegen;
pub mod compile_service_canonical;
pub mod ops;
pub mod template;

pub use analysis::*;
pub use canonical_air::*;
pub use canonical_execute::*;
pub use cli::*;
pub use codegen::*;
pub use ops::*;
pub use template::*;

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
