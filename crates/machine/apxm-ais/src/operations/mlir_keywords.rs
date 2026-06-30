//! Canonical MLIR syntactic-keyword literals used in AIS operation
//! `assemblyFormat` specs.
//!
//! These are the literal tokens that appear between operands in an op's
//! printed form (e.g. `ais.communicate "msg" to "recipient"`). Every literal
//! keyword referenced from a `MlirEmissionSpec::syntactic_keywords` entry
//! MUST come from this module — no raw string literals.

/// `to` — preposition for COMMUNICATE (`$message to $recipient`) and DELEGATE
/// (`$task_spec to $target_agent`).
pub const TO: &str = "to";
