//! Gate 3 — one capability id list.
//!
//! Plan gate 3 requires the compiler tool-binding allowlist (`BUILTINS`), the
//! runtime registrar (`register_standard_tools`), and the id each implemented
//! builtin reports as its own name to agree. Before this test existed those
//! three sets disagreed in five ways: two allowlisted ids implemented
//! nowhere (`capability_discovery`, `manage_task`), two implemented-and-
//! registered ids (`http_get`, `http_post`) never actually registered, one
//! registered id (`count_tokens`) missing from the allowlist so
//! `Tool("count_tokens")` was admitted at runtime but rejected by
//! `apxm agent lint`, two implemented ids (`mcp.call`, `provider.call`) with
//! no canonical constant at all, and three allowlisted ids
//! (`list_local_skills`, `search_skills`, `read_local_skill`) with no
//! handler behind them whatsoever.
//!
//! Two gaps are real architecture, not drift, and this test pins them by
//! name rather than papering over them:
//! - `SCHEDULE` is durable-backend-only: no runtime profile in this crate
//!   registers it, so it is allowlisted but absent from both the registered
//!   set and the implemented set.
//! - `MCP_CALL` / `PROVIDER_CALL` are implemented and allowlisted, but
//!   `register_standard_tools` never registers their base ids — a pack
//!   entry mints a per-tool/per-action id dynamically via `named()` /
//!   `named_rest()` instead (see `apxm_ais::capabilities::BUILTINS` doc
//!   comment). That generalization is out of scope here; this test only
//!   requires the base ids to be coherent.
//!
//! Any *other* desync — a new phantom allowlist id, an implemented builtin
//! missing from the allowlist, a registered capability the allowlist would
//! reject — fails one of the assertions below.

use apxm_capability::CapabilitySystem;
use apxm_capability::builtins::{
    BashCapability, CountTokensCapability, HttpGetCapability, HttpPostCapability,
    McpBridgeCapability, ProviderCallCapability, ReadCapability, SearchWebCapability, ToolsConfig,
    WriteCapability, register_standard_tools,
};
use apxm_capability::executor::CapabilityExecutor;
use apxm_core::constants::capabilities::{BUILTINS, MCP_CALL, PROVIDER_CALL, SCHEDULE, STANDARD_BUILTINS};
use std::collections::BTreeSet;

fn set_of(items: &[&str]) -> BTreeSet<String> {
    items.iter().map(|s| (*s).to_string()).collect()
}

/// The ids `register_standard_tools` actually registers with every `enabled`
/// flag at its default (`true`). This is the "registered" third of gate 3.
fn registered_ids() -> BTreeSet<String> {
    let system = CapabilitySystem::new();
    register_standard_tools(&system, &ToolsConfig::default()).expect("standard tools register");
    system.list_capability_names().into_iter().collect()
}

/// The literal id each implemented builtin capability reports via its own
/// `CapabilityExecutor::metadata()`, independent of whether anything
/// registers it. This is the "ids builtins report" third of gate 3. Every
/// builtin implementation this crate ships belongs in this list.
fn implemented_ids() -> BTreeSet<String> {
    let caps: Vec<Box<dyn CapabilityExecutor>> = vec![
        Box::new(BashCapability::new()),
        Box::new(ReadCapability::new()),
        Box::new(WriteCapability::new()),
        Box::new(SearchWebCapability::new()),
        Box::new(HttpGetCapability::new()),
        Box::new(HttpPostCapability::new()),
        Box::new(CountTokensCapability::new()),
        Box::new(McpBridgeCapability::new()),
        Box::new(ProviderCallCapability::new()),
    ];
    caps.iter().map(|c| c.metadata().name.clone()).collect()
}

#[test]
fn registered_set_equals_standard_builtins() {
    // Registrar half of gate 3: everything `register_standard_tools`
    // registers is exactly `STANDARD_BUILTINS` — no id registers without a
    // constant naming it there, and no constant claims a registration that
    // does not happen.
    assert_eq!(registered_ids(), set_of(STANDARD_BUILTINS));
}

#[test]
fn every_implemented_id_matches_a_canonical_constant_and_is_allowlisted() {
    // "ids builtins report" half of gate 3: every implemented builtin's own
    // metadata name is in `BUILTINS`. This is exactly the `count_tokens`
    // trap: registered/implemented but rejected by `agent lint`.
    let implemented = implemented_ids();
    let allowlist = set_of(BUILTINS);
    for id in &implemented {
        assert!(
            allowlist.contains(id.as_str()),
            "builtin capability reports id '{id}' but it is missing from BUILTINS"
        );
    }
}

#[test]
fn standard_builtins_is_a_subset_of_the_allowlist() {
    let standard = set_of(STANDARD_BUILTINS);
    let allowlist = set_of(BUILTINS);
    for id in &standard {
        assert!(
            allowlist.contains(id.as_str()),
            "'{id}' is registered by register_standard_tools but missing from BUILTINS"
        );
    }
}

#[test]
fn implemented_set_equals_standard_plus_dynamic_backings() {
    // Ties the registrar set and the "ids builtins report" set together:
    // every implemented builtin is either always-registered (in
    // STANDARD_BUILTINS) or one of the two dynamic per-pack backings
    // (mcp.call, provider.call). No implemented builtin is orphaned from
    // both classification paths.
    let implemented = implemented_ids();
    let mut expected = set_of(STANDARD_BUILTINS);
    expected.insert(MCP_CALL.to_string());
    expected.insert(PROVIDER_CALL.to_string());
    assert_eq!(implemented, expected);
}

#[test]
fn allowlist_residual_beyond_registered_and_implemented_is_exactly_the_durable_set() {
    // The one gap gate 3 cannot close from a single call site: `SCHEDULE` is
    // real — registered only by a runtime profile with a durable
    // persistence backend that does not exist in this crate. Pin the
    // residual to exactly that one, documented id so any *other* phantom id
    // sneaking into BUILTINS (the failure mode this whole test file exists
    // to catch) fails here instead of silently passing lint.
    let allowlist = set_of(BUILTINS);
    let mut covered = set_of(STANDARD_BUILTINS);
    covered.insert(MCP_CALL.to_string());
    covered.insert(PROVIDER_CALL.to_string());
    let residual: BTreeSet<String> = allowlist.difference(&covered).cloned().collect();
    assert_eq!(residual, set_of(&[SCHEDULE]));
}
