//! Compatibility module for the public program grammar path.
//!
//! The closed primitives are owned by `apxm-core`; this module preserves the
//! existing `apxm_program::grammar` API without keeping a second definition.

pub use apxm_core::grammar::{is_digest, is_identifier, is_schema_id};
