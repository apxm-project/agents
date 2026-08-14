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
//! That last desync was closed by deleting the ids, and is now closed the other
//! way: `list_skills`, `search_skills`, and `read_skill` are back in the
//! allowlist *because* `apxm_capability::builtins::skills` implements them, and
//! they appear in `implemented_ids()` below, which is what makes the re-add
//! legitimate rather than a repeat. Deleting the handlers without deleting the
//! ids fails `every_implemented_id_matches_a_canonical_constant_and_is_allowlisted`
//! and `implemented_set_equals_standard_plus_dynamic_backings`.
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
    ListSkillsCapability, McpBridgeCapability, ProviderCallCapability, ReadCapability,
    ReadSkillCapability, SearchSkillsCapability, SearchWebCapability, ToolsConfig, WriteCapability,
    register_standard_tools,
};
use apxm_capability::executor::CapabilityExecutor;
use apxm_core::constants::capabilities::{
    BUILTIN_GROUPS, BUILTINS, MCP_CALL, PROVIDER_CALL, SCHEDULE, STANDARD_BUILTINS, groups,
};
use std::collections::{BTreeMap, BTreeSet};

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
        Box::new(ListSkillsCapability::new()),
        Box::new(SearchSkillsCapability::new()),
        Box::new(ReadSkillCapability::new()),
    ];
    caps.iter().map(|c| c.metadata().name.clone()).collect()
}

/// The group tags each implemented builtin reports, inverted into
/// group -> members. `builtin_group` is a name an author may declare in a
/// package, so a group with no member capability is the same failure as an
/// allowlisted id with no handler: it lints clean and binds nothing.
fn group_members() -> BTreeMap<String, BTreeSet<String>> {
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
        Box::new(ListSkillsCapability::new()),
        Box::new(SearchSkillsCapability::new()),
        Box::new(ReadSkillCapability::new()),
    ];
    let mut members: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for capability in &caps {
        let metadata = capability.metadata();
        for group in &metadata.groups {
            members
                .entry(group.clone())
                .or_default()
                .insert(metadata.name.clone());
        }
    }
    members
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

/// `builtin_group = "skills"` used to resolve to nothing: the group was in
/// `BUILTIN_GROUPS`, so `agent lint` accepted a package declaring it, and no
/// capability carried the tag, so it bound nothing at load. That is the
/// allowlist-without-handler failure one level up, and this pins it closed.
///
/// The remaining empty groups are named individually rather than tolerated as a
/// class, so the next group to gain a member shrinks this list in review
/// instead of silently passing.
#[test]
fn declared_builtin_groups_resolve_to_member_capabilities() {
    let members = group_members();
    for group in [groups::SKILLS, groups::DISCOVERY] {
        let bound = members.get(group);
        assert!(
            bound.is_some_and(|ids| !ids.is_empty()),
            "builtin group '{group}' is declarable but no capability carries it"
        );
    }
    assert_eq!(
        members.get(groups::SKILLS),
        Some(&set_of(&["list_skills", "read_skill", "search_skills"])),
        "the skills group is exactly the three implemented skill capabilities"
    );

    let unbound: BTreeSet<&str> = BUILTIN_GROUPS
        .iter()
        .copied()
        .filter(|group| !members.contains_key(*group))
        .collect();
    assert_eq!(
        unbound,
        BTreeSet::from([groups::AUTHORING, groups::TASK, groups::AGENT_MANAGEMENT]),
        "an author may declare these groups today and bind nothing; that is the \
         residual this test exists to keep visible and shrinking"
    );
}
