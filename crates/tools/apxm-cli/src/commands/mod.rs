//! Command modules for APXM CLI.

pub mod cli;
pub mod implementations;

pub mod agent;
pub mod analysis;
pub mod backend;
pub mod cache;
pub mod codegen;
pub mod compile;
pub mod execute;
pub mod gui;
pub mod ops;
pub mod replay;
pub mod session;
pub mod system;
pub mod task;
pub mod team;
pub mod template;
pub mod tool;
pub mod workflow;

// Re-export CLI types
pub use cli::*;

// Re-export shared helpers and command functions
pub use implementations::*;

pub use agent::*;
pub use analysis::*;
pub use backend::*;
pub use cache::*;
pub use codegen::*;
pub use compile::*;
pub use execute::*;
pub use gui::*;
pub use ops::*;
pub use replay::*;
pub use session::*;
pub use system::*;
pub use task::*;
pub use team::*;
pub use template::*;
pub use tool::*;
pub use workflow::*;
