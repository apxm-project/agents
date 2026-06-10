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

/// Event-driven scheduling tool (durable one-shot / recurring wakeups).
pub const SCHEDULE: &str = "schedule";
/// Task/goal management tool (CRUD over the AAM goal tree).
pub const MANAGE_TASK: &str = "manage_task";

/// Well-known memory capability ids (server memory routes / per-agent stores).
pub const MEMORY_STORE_FACT: &str = "memory.store_fact";
pub const MEMORY_SEARCH_FACTS: &str = "memory.search_facts";

/// Tool-group tags used by capability metadata and LLM tool exposure.
pub mod groups {
    pub const FILE: &str = "file";
    pub const FILE_READ: &str = "file:read";
    pub const FILE_WRITE: &str = "file:write";
    pub const HTTP: &str = "http";
    pub const WEB: &str = "web";
    pub const TASK: &str = "task";
    pub const AGENT_MANAGEMENT: &str = "agent_management";
}

/// Core local tools that can exist without durable agent-management storage.
pub const STANDARD_BUILTINS: &[&str] = &[BASH, READ, WRITE, SEARCH_WEB, HTTP_GET, HTTP_POST];

/// Durable agent-management builtins. Runtime crates register these only when
/// their persistence backend is available.
pub const AGENT_MANAGEMENT_BUILTINS: &[&str] = &[SCHEDULE, MANAGE_TASK];

/// Built-in capabilities admitted without explicit registration (compiler
/// tool-binding allowlist and runtime startup registration use this set).
pub const BUILTINS: &[&str] = &[
    BASH,
    READ,
    WRITE,
    SEARCH_WEB,
    HTTP_GET,
    HTTP_POST,
    SCHEDULE,
    MANAGE_TASK,
];
