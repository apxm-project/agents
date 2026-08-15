//! Dekk-owned compiler and runtime fixture library.

#![allow(dead_code)]

pub mod commands;
pub mod frontend;

pub use commands::canonical_execute;
