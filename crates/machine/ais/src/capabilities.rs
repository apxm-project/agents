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
/// Pure/read-only token-size estimator; host-independent, registered on every
/// runtime profile alongside the standard tools.
pub const COUNT_TOKENS: &str = "count_tokens";
/// MCP-client bridge: `tools/call` over Streamable HTTP against an external
/// MCP server. Base id for the generic block; `named()` mints per-tool ids
/// for a declared `kind = "mcp"` pack entry.
pub const MCP_CALL: &str = "mcp.call";
/// Generic authenticated outbound HTTP call, forwarded through apxm-auth's
/// proxy. Base id for the generic block; `named()`/`named_rest()` mint
/// per-action ids for a declared `kind = "provider"` pack entry.
pub const PROVIDER_CALL: &str = "provider.call";

// `capability_discovery` used to live here too. It had no implementation
// anywhere — superseded by the generic `builtin_group = "discovery"` grouping,
// see `groups::DISCOVERY` / `BUILTIN_GROUPS` below — and was deleted outright.
//
// The three skill-discovery ids below were also removed once, for a different
// reason: they were allowlisted with no handler behind them, so `agent lint`
// accepted a package declaring a capability that could never execute. They are
// back because `apxm_capability::builtins::skills` implements them, under the
// settled name `read_skill` (not `read_local_skill`) and its list/search
// siblings. That ordering — handler first, id second — is the rule the earlier
// state broke, and the gate-3 inventory test in
// `crates/runtime/capability/tests/` is what holds it: an id here with no
// implementation reporting it fails that test.

/// Metadata-only listing of the skills a configured discovery root publishes.
/// Listing never loads an instruction body.
pub const LIST_SKILLS: &str = "list_skills";
/// Metadata-only search over the same discovery cards `LIST_SKILLS` returns.
pub const SEARCH_SKILLS: &str = "search_skills";
/// Load exactly one skill's instruction body by id. The only one of the three
/// that activates a skill rather than advertising it.
pub const READ_SKILL: &str = "read_skill";

/// Event-driven scheduling tool (durable one-shot / recurring wakeups).
/// Unimplemented in this crate today; registered only by a runtime profile
/// with a durable persistence backend, so it stays out of `STANDARD_BUILTINS`
/// and out of `register_standard_tools`. Left in `BUILTINS` so an authored
/// package targeting such a profile is not rejected at lint time.
pub const SCHEDULE: &str = "schedule";
/// Task/goal management tool (CRUD over the AAM goal tree). Same
/// durable-backend caveat as `SCHEDULE`. Unlike `SCHEDULE`, this id is a
/// legacy artifact with no committed durable-backend plan, so it is removed
/// from `BUILTINS`; the constant and its `AGENT_MANAGEMENT_BUILTINS` slot
/// stay for that future backend to reintroduce it there, not here.
pub const MANAGE_TASK: &str = "manage_task";

/// Tool-group tags for capability metadata and LLM tool exposure.
pub mod groups {
    pub const FILE: &str = "file";
    pub const FILE_READ: &str = "file:read";
    pub const FILE_WRITE: &str = "file:write";
    pub const READ: &str = "read";
    pub const WRITE: &str = "write";
    pub const HTTP: &str = "http";
    pub const WEB: &str = "web";
    pub const SEARCH: &str = "search";
    pub const WEB_SEARCH: &str = "web:search";
    pub const DISCOVERY: &str = "discovery";
    /// Members: `list_skills`, `search_skills`, `read_skill`. Declaring this
    /// group used to resolve to no capability at all — it lint-checked clean
    /// and bound nothing — which is the same fraud as an allowlisted id with
    /// no handler, one level up. The membership test in
    /// `crates/runtime/capability/tests/` now holds it non-empty.
    pub const SKILLS: &str = "skills";
    pub const AUTHORING: &str = "authoring";
    pub const TASK: &str = "task";
    pub const AGENT_MANAGEMENT: &str = "agent_management";
    pub const TEXT: &str = "text";
}

/// Core local tools that can exist without durable agent-management storage.
/// Exactly the ids `register_standard_tools` registers (each behind its own
/// `enabled` config flag, except `count_tokens` which is unconditional) —
/// this is the SSOT the gate-3 inventory test checks the registrar against.
pub const STANDARD_BUILTINS: &[&str] = &[
    BASH,
    READ,
    WRITE,
    SEARCH_WEB,
    HTTP_GET,
    HTTP_POST,
    COUNT_TOKENS,
    LIST_SKILLS,
    SEARCH_SKILLS,
    READ_SKILL,
];

/// Durable agent-management builtins. Runtime crates register these only when
/// their persistence backend is available.
pub const AGENT_MANAGEMENT_BUILTINS: &[&str] = &[SCHEDULE, MANAGE_TASK];

/// Built-in capabilities admitted without explicit registration (compiler
/// tool-binding allowlist and runtime startup registration use this set).
///
/// This is `STANDARD_BUILTINS` (always registered) plus `MCP_CALL` /
/// `PROVIDER_CALL` (implemented, but registered dynamically per pack entry
/// rather than by `register_standard_tools` — see their `named()` /
/// `named_rest()` constructors) plus `SCHEDULE` (durable-backend only, see
/// its doc comment above). Every other builtin id `register_standard_tools`
/// can register, and every id an implemented builtin capability reports as
/// its own name, is in this list — that equivalence is gate 3, pinned by
/// the inventory test in `crates/runtime/capability/tests/`.
pub const BUILTINS: &[&str] = &[
    BASH,
    READ,
    WRITE,
    SEARCH_WEB,
    HTTP_GET,
    HTTP_POST,
    COUNT_TOKENS,
    LIST_SKILLS,
    SEARCH_SKILLS,
    READ_SKILL,
    MCP_CALL,
    PROVIDER_CALL,
    SCHEDULE,
];

/// Valid `builtin_group` values for grouped `kind = "builtin"` entries in an
/// agent folder. This is the single source of truth the CLI `agent lint`
/// capability check uses so a mistyped builtin group is rejected at author time
/// rather than deferred to load. Keep in sync with the server's builtin-group
/// projection.
pub const BUILTIN_GROUPS: &[&str] = &[
    groups::SKILLS,
    groups::AUTHORING,
    groups::DISCOVERY,
    groups::TASK,
    groups::AGENT_MANAGEMENT,
];
