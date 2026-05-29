//! Canonical capability identifiers.
//!
//! These are the stable string ids for built-in capabilities and the well-known
//! memory tools. They are defined here, in the single-source-of-truth crate, so
//! the compiler allowlist, the runtime registration, and authoring front-ends
//! all agree without re-introducing raw string literals.

pub const BASH: &str = "bash";
pub const READ: &str = "read";
pub const WRITE: &str = "write";
pub const SEARCH_WEB: &str = "search_web";
pub const HTTP_GET: &str = "http_get";
pub const HTTP_POST: &str = "http_post";

/// Well-known memory capability ids (server memory routes / per-agent stores).
pub const MEMORY_STORE_FACT: &str = "memory.store_fact";
pub const MEMORY_SEARCH_FACTS: &str = "memory.search_facts";

/// Built-in capabilities admitted without explicit registration (compiler
/// tool-binding allowlist and runtime startup registration use this set).
pub const BUILTINS: &[&str] = &[BASH, READ, WRITE, SEARCH_WEB, HTTP_GET, HTTP_POST];
