//! `apxm org new|lint|install` — the toolchain for the canonical
//! organization-package folder format (`apxm.org-package` /
//! `apxm.org-topology`).
//!
//! This module is the org-level sibling of [`super::agent`]: same
//! module organization, same manifest-projection-from-schema approach (hand
//! ported into Rust structs rather than loading the JSON schema at runtime —
//! this repo does not vendor or path-depend on the sibling `contracts`
//! repo), same lint reporting style, same install-to-`APXM_HOME` pattern.
//! Where a check can reuse the agent command logic (installed-agent resolution,
//! the `[hierarchy]` parent/permitted_children shape, recursive directory copy)
//! it does — see the `use super::agent::{...}` imports below — rather than
//! reimplementing it. The joined capabilities/permissions grammar is *not*
//! shared: it is the org format's alone now that an agent package declares no
//! capability inventory of its own.
//!
//! An org package's `members.toml` entries carry a `hierarchy` snapshot
//! (org-package.v1#/properties/members/items/properties/hierarchy) of the
//! referenced agent's own `agent.toml [hierarchy]`. That snapshot — not a
//! second resolve-and-read of the installed agent's manifest — is
//! what `org lint`'s hierarchy-consistency check compares against
//! `topology.toml`'s tree edges; this is the schema's own design ("carried
//! here so org-package lint can check consistency ... without
//! resolving the referenced agent") and matches the
//! `invalid-member-hierarchy-contradicts-topology-tree` vector, which
//! exercises exactly this snapshot-vs-tree contradiction with no reference
//! to an installed agent at all.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow, bail};
use serde::{Deserialize, Serialize};

use super::agent::{HierarchyToml, copy_dir_recursive, resolve_installed_agent};
use super::implementations::{Status, print_section_header, print_status_line};

// ---------------------------------------------------------------------
// On-disk manifest shapes (org-package.v1 / org-topology.v1 projections)
// ---------------------------------------------------------------------

/// `capabilities/capabilities.toml` — the org's GLOBAL capability set.
///
/// This shape is the org format's alone. An agent package declares no
/// capability inventory: what it can supply is the built-in allowlist plus the
/// handlers it ships, so there is nothing at the agent level for these types to
/// be shared with.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CapabilitiesToml {
    #[serde(default, rename = "capability")]
    pub capability: Vec<CapabilityEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CapabilityEntry {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(flatten)]
    pub extra: toml::Table,
}

/// `capabilities/permissions.toml` — one policy entry per joined global
/// capability id. A global with no matching policy is not a capability and
/// fails `org lint`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PermissionsToml {
    #[serde(default, rename = "permission")]
    pub permission: Vec<PermissionEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PermissionEntry {
    pub capability: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub decision: Option<toml::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(flatten)]
    pub extra: toml::Table,
}

/// The org-global joined capability set (declared ∩ permitted — the same join
/// `check_global_capability_join` enforces), read for `agent lint --org` so a
/// member package's `agent.toml [permissions]` may state a decision for a
/// capability the org supplies.
///
/// Missing files are an empty global set rather than an error: this is opt-in
/// plumbing for a package that is a member of an org, not a requirement every
/// org must satisfy.
pub(super) fn load_org_global_capabilities(org_root: &Path) -> Result<BTreeSet<String>> {
    let capabilities_path = org_root.join("capabilities/capabilities.toml");
    let permissions_path = org_root.join("capabilities/permissions.toml");
    let capabilities: CapabilitiesToml = if capabilities_path.is_file() {
        read_toml(&capabilities_path)?
    } else {
        CapabilitiesToml::default()
    };
    let permissions: PermissionsToml = if permissions_path.is_file() {
        read_toml(&permissions_path)?
    } else {
        PermissionsToml::default()
    };
    let declared: BTreeSet<&str> = capabilities
        .capability
        .iter()
        .map(|c| c.id.as_str())
        .collect();
    let permitted: BTreeSet<&str> = permissions
        .permission
        .iter()
        .map(|p| p.capability.as_str())
        .collect();
    Ok(declared
        .intersection(&permitted)
        .map(|id| (*id).to_string())
        .collect())
}

/// Projection of `org.toml` (`org-package.v1#/properties/org`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrgToml {
    pub org_id: String,
    pub display_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub policy: Option<toml::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub environment: Option<toml::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub models: Option<toml::Value>,
}

/// Projection of `agents/members.toml` (`org-package.v1#/properties/members`).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MembersToml {
    #[serde(default, rename = "member")]
    pub member: Vec<MemberEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemberEntry {
    pub id: String,
    pub package: String,
    /// SemVer requirement against the referenced agent's own `agent.toml`
    /// version (exact version, or a `^`/`~` range).
    pub version: String,
    #[serde(default = "default_instances")]
    pub instances: u32,
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub capability_mask: Option<CapabilityMaskToml>,
    /// Snapshot of the referenced agent's own `[hierarchy]` table
    /// (`apxm.agent#/$defs/Hierarchy`). Reused verbatim from
    /// [`super::agent::HierarchyToml`] — same shape.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hierarchy: Option<HierarchyToml>,
}

fn default_instances() -> u32 {
    1
}

fn default_enabled() -> bool {
    true
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CapabilityMaskToml {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub grant: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub deny: Vec<String>,
}

/// Projection of `topology.toml`, conforming to `apxm.org-topology`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TopologyToml {
    pub tree: TreeToml,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub relations: Vec<RelationToml>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub directory_policy: Option<toml::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub handoff_policy: Option<toml::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TreeToml {
    pub root: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub edges: Vec<EdgeToml>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EdgeToml {
    pub parent: String,
    pub child: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RelationToml {
    pub from: String,
    pub to: String,
    pub r#type: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reach: Option<String>,
}

// ---------------------------------------------------------------------
// org new
// ---------------------------------------------------------------------

pub fn org_command(action: super::OrgAction, json_output: bool) -> Result<()> {
    match action {
        super::OrgAction::New {
            id,
            path,
            display_name,
        } => org_new(&id, path, display_name, json_output),
        super::OrgAction::Lint { path } => org_lint(&path, json_output),
        super::OrgAction::Install { path, force } => org_install(&path, force, json_output),
    }
}

fn default_org_root(id: &str) -> PathBuf {
    PathBuf::from("orgs").join(id)
}

fn titleize(id: &str) -> String {
    id.split(['-', '_'])
        .filter(|s| !s.is_empty())
        .map(|word| {
            let mut chars = word.chars();
            match chars.next() {
                Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn write_new_file(path: &Path, contents: &str) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("Failed to create {}", parent.display()))?;
    }
    fs::write(path, contents).with_context(|| format!("Failed to write {}", path.display()))
}

fn org_new(
    id: &str,
    path: Option<PathBuf>,
    display_name: Option<String>,
    json_output: bool,
) -> Result<()> {
    if id.trim().is_empty() {
        bail!("org id must not be empty");
    }
    let root = path.unwrap_or_else(|| default_org_root(id));
    if root.exists() && fs::read_dir(&root)?.next().is_some() {
        bail!(
            "destination '{}' already exists and is not empty",
            root.display()
        );
    }

    let display_name = display_name.unwrap_or_else(|| titleize(id));
    let placeholder_root_member = format!("{id}-root");

    // org.toml
    let org_toml = format!(
        "org_id = \"{id}\"\n\
         display_name = \"{display_name}\"\n\
         description = \"Describe this org's purpose here.\"\n\n\
         [policy]\n\
         approval_default = \"manual\"\n\
         autonomy_ceiling = \"supervised\"\n"
    );
    write_new_file(&root.join("org.toml"), &org_toml)?;

    // capabilities/ — the GLOBAL capability set shared by every member
    // unless masked by a member's capability_mask.
    write_new_file(
        &root.join("capabilities/capabilities.toml"),
        "# capability-definition.v1 entries — the org's GLOBAL capability set.\n\
         # Every entry here must have a matching capabilities/permissions.toml\n\
         # entry (the joined capability) or `apxm org lint` fails.\n",
    )?;
    write_new_file(
        &root.join("capabilities/permissions.toml"),
        "# One [[permission]] per capabilities.toml entry (by id).\n",
    )?;

    // agents/members.toml — empty by default; author adds [[member]] blocks
    // that reference real, installed apxm.agent agents.
    write_new_file(
        &root.join("agents/members.toml"),
        "# One [[member]] per org member, by reference to an installed\n\
         # agent (apxm.agent). No agent code lives here.\n\
         #\n\
         # [[member]]\n\
         # id = \"root-agent\"\n\
         # package = \"some.agent\"\n\
         # version = \"0.1.0\"\n\
         #\n\
         # [member.capability_mask]\n\
         # deny = [\"some.capability\"]\n\
         #\n\
         # [member.hierarchy]\n\
         # permitted_children = [\"child-agent\"]\n",
    )?;

    // topology.toml — a single-node placeholder tree; update `root`/`edges`
    // to match the members declared above.
    let topology_toml = format!(
        "[tree]\n\
         root = \"{placeholder_root_member}\"\n\
         edges = []\n\n\
         # One [[relations]] entry per pairwise topology relation.\n\
         # [[relations]]\n\
         # from = \"root-agent\"\n\
         # to = \"child-agent\"\n\
         # type = \"delegates_to\"\n"
    );
    write_new_file(&root.join("topology.toml"), &topology_toml)?;

    // prompts/ and tests/ — optional, scaffolded for parity with `agent new`.
    write_new_file(
        &root.join("prompts/persona.md"),
        &format!("# {display_name}\n\nDescribe this org's collective persona/voice here.\n"),
    )?;
    write_new_file(
        &root.join("tests/README.md"),
        "Org-package tests (run by the linter/CI) go here.\n",
    )?;

    if json_output {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "id": id,
                "path": root.display().to_string(),
                "status": "scaffolded",
            }))?
        );
    } else {
        print_section_header("Org Scaffolded");
        print_status_line(id, Status::Ok, &root.display().to_string());
        println!();
        println!("Next steps:");
        println!("  apxm org lint {}", root.display());
        println!("  apxm org install {}", root.display());
    }
    Ok(())
}

// ---------------------------------------------------------------------
// Shared manifest loading
// ---------------------------------------------------------------------

fn read_toml<T: serde::de::DeserializeOwned>(path: &Path) -> Result<T> {
    let text =
        fs::read_to_string(path).with_context(|| format!("Failed to read {}", path.display()))?;
    toml::from_str(&text).with_context(|| format!("Failed to parse {}", path.display()))
}

struct LoadedOrg {
    root: PathBuf,
    org: OrgToml,
    capabilities: CapabilitiesToml,
    permissions: PermissionsToml,
    members: MembersToml,
    topology: TopologyToml,
}

fn load_org(root: &Path) -> Result<LoadedOrg> {
    if !root.is_dir() {
        bail!("'{}' is not a directory", root.display());
    }
    let org_path = root.join("org.toml");
    if !org_path.is_file() {
        bail!("missing required file: {}", org_path.display());
    }
    let org: OrgToml = read_toml(&org_path)?;

    let members_path = root.join("agents/members.toml");
    if !members_path.is_file() {
        bail!("missing required file: {}", members_path.display());
    }
    let members: MembersToml = read_toml(&members_path)?;

    let topology_path = root.join("topology.toml");
    if !topology_path.is_file() {
        bail!("missing required file: {}", topology_path.display());
    }
    let topology: TopologyToml = read_toml(&topology_path)?;

    let capabilities_path = root.join("capabilities/capabilities.toml");
    let capabilities = if capabilities_path.is_file() {
        read_toml(&capabilities_path)?
    } else {
        CapabilitiesToml::default()
    };
    let permissions_path = root.join("capabilities/permissions.toml");
    let permissions = if permissions_path.is_file() {
        read_toml(&permissions_path)?
    } else {
        PermissionsToml::default()
    };

    Ok(LoadedOrg {
        root: root.to_path_buf(),
        org,
        capabilities,
        permissions,
        members,
        topology,
    })
}

// ---------------------------------------------------------------------
// org lint
// ---------------------------------------------------------------------

/// Minimal SemVer core parse (`MAJOR.MINOR.PATCH`, prerelease/build metadata
/// ignored) — uses `agent.rs::semver_like`'s tolerance, adding the
/// numeric triple needed for requirement satisfaction.
fn parse_semver_core(version: &str) -> Option<(u64, u64, u64)> {
    let core = version.split(['-', '+']).next().unwrap_or(version);
    let parts: Vec<&str> = core.split('.').collect();
    if parts.len() != 3 {
        return None;
    }
    let major = parts[0].parse().ok()?;
    let minor = parts[1].parse().ok()?;
    let patch = parts[2].parse().ok()?;
    Some((major, minor, patch))
}

/// Does `actual` satisfy the `requirement` string from a member's
/// `version` field? Supports exact match, `^` (compatible: same major,
/// `>=` requirement), and `~` (same major.minor, `>=` requirement).
/// Unparseable requirements/versions fall back to an exact string
/// comparison — no `semver` crate dependency exists in this workspace
/// today (see `semver_like`, which only validates shape).
fn version_satisfies(requirement: &str, actual: &str) -> bool {
    let requirement = requirement.trim();
    if let Some(rest) = requirement.strip_prefix('^') {
        return match (parse_semver_core(rest), parse_semver_core(actual)) {
            (Some(req), Some(act)) => act.0 == req.0 && act >= req,
            _ => requirement == actual,
        };
    }
    if let Some(rest) = requirement.strip_prefix('~') {
        return match (parse_semver_core(rest), parse_semver_core(actual)) {
            (Some(req), Some(act)) => act.0 == req.0 && act.1 == req.1 && act >= req,
            _ => requirement == actual,
        };
    }
    match (parse_semver_core(requirement), parse_semver_core(actual)) {
        (Some(req), Some(act)) => req == act,
        _ => requirement == actual,
    }
}

/// Check 1: every member reference must resolve to an installed
/// agent (`APXM_HOME/agents/<id>/`, 's install layout) whose
/// version satisfies the member's version requirement.
fn check_member_resolution(members: &MembersToml, apxm_home: &Path) -> Vec<String> {
    let mut errors = Vec::new();
    for member in &members.member {
        match resolve_installed_agent(apxm_home, &member.package) {
            Ok(pack) => {
                if !version_satisfies(&member.version, &pack.version) {
                    errors.push(format!(
                        "member '{}' requires package '{}' version '{}', but the installed \
                         package is at version '{}'",
                        member.id, member.package, member.version, pack.version
                    ));
                }
            }
            Err(err) => {
                errors.push(format!(
                    "member '{}' references package '{}', which could not be resolved: {err}",
                    member.id, member.package
                ));
            }
        }
    }
    errors
}

/// Check 2: `topology.toml`'s tree must have exactly one root and
/// no cycles. Real cycle detection via DFS coloring, not just "trust the
/// format".
fn check_tree_well_formed(tree: &TreeToml) -> Vec<String> {
    let mut errors = Vec::new();

    if tree.root.trim().is_empty() {
        errors.push("topology.tree: root must not be empty".to_string());
        return errors;
    }

    let mut children_of: HashMap<&str, Vec<&str>> = HashMap::new();
    let mut parent_of: HashMap<&str, &str> = HashMap::new();
    let mut nodes: BTreeSet<&str> = BTreeSet::new();
    nodes.insert(tree.root.as_str());

    for edge in &tree.edges {
        nodes.insert(edge.parent.as_str());
        nodes.insert(edge.child.as_str());
        children_of
            .entry(edge.parent.as_str())
            .or_default()
            .push(edge.child.as_str());
        if let Some(existing_parent) = parent_of.insert(edge.child.as_str(), edge.parent.as_str())
            && existing_parent != edge.parent.as_str()
        {
            errors.push(format!(
                "topology.tree: node '{}' has multiple parents ('{}' and '{}') — not a tree",
                edge.child, existing_parent, edge.parent
            ));
        }
    }

    // Single root: exactly one node with no incoming edge, and it must be
    // the declared `root`.
    let roots: Vec<&str> = nodes
        .iter()
        .filter(|n| !parent_of.contains_key(*n))
        .copied()
        .collect();
    if roots.len() != 1 {
        errors.push(format!(
            "topology.tree: expected exactly one root (node with no parent edge), found {}: {:?}",
            roots.len(),
            roots
        ));
    } else if roots[0] != tree.root {
        errors.push(format!(
            "topology.tree: declared root '{}' does not match the tree's actual root '{}'",
            tree.root, roots[0]
        ));
    }

    // Cycle detection: DFS from the declared root, tracking the recursion
    // stack; any edge back into the stack is a cycle. Also verify every
    // node is reachable from `root` exactly once (a well-formed tree).
    #[derive(PartialEq)]
    enum Color {
        InProgress,
        Done,
    }
    let mut colors: HashMap<&str, Color> = HashMap::new();
    let mut visited_count = 0usize;
    let mut path: Vec<&str> = Vec::new();

    fn dfs<'a>(
        node: &'a str,
        children_of: &HashMap<&'a str, Vec<&'a str>>,
        colors: &mut HashMap<&'a str, Color>,
        path: &mut Vec<&'a str>,
        visited_count: &mut usize,
        errors: &mut Vec<String>,
    ) {
        if let Some(existing) = colors.get(node) {
            if *existing == Color::InProgress {
                path.push(node);
                errors.push(format!(
                    "topology.tree: cycle detected involving node '{}' (path: {})",
                    node,
                    path.join(" -> ")
                ));
                path.pop();
            }
            return;
        }
        colors.insert(node, Color::InProgress);
        path.push(node);
        *visited_count += 1;
        if let Some(children) = children_of.get(node) {
            for child in children {
                dfs(child, children_of, colors, path, visited_count, errors);
            }
        }
        path.pop();
        colors.insert(node, Color::Done);
    }

    dfs(
        tree.root.as_str(),
        &children_of,
        &mut colors,
        &mut path,
        &mut visited_count,
        &mut errors,
    );

    if visited_count < nodes.len() && errors.iter().all(|e| !e.contains("cycle detected")) {
        let unreached: Vec<&str> = nodes
            .iter()
            .filter(|n| !colors.contains_key(*n))
            .copied()
            .collect();
        if !unreached.is_empty() {
            errors.push(format!(
                "topology.tree: node(s) {unreached:?} are not reachable from root '{}' (disconnected forest, not a single tree)",
                tree.root
            ));
        }
    }

    errors
}

/// Check 3: a member's own `hierarchy` snapshot (parent /
/// permitted_children shape) must not contradict `topology.tree`'s
/// edges. Uses the
/// `invalid-member-hierarchy-contradicts-topology-tree` vector.
fn check_hierarchy_consistency(members: &MembersToml, tree: &TreeToml) -> Vec<String> {
    let mut errors = Vec::new();

    let mut parent_of: HashMap<&str, &str> = HashMap::new();
    let mut children_of: HashMap<&str, BTreeSet<&str>> = HashMap::new();
    for edge in &tree.edges {
        parent_of.insert(edge.child.as_str(), edge.parent.as_str());
        children_of
            .entry(edge.parent.as_str())
            .or_default()
            .insert(edge.child.as_str());
    }

    for member in &members.member {
        let Some(hierarchy) = &member.hierarchy else {
            continue;
        };
        if let Some(declared_parent) = &hierarchy.parent {
            match parent_of.get(member.id.as_str()) {
                Some(actual_parent) if actual_parent == declared_parent => {}
                Some(actual_parent) => {
                    errors.push(format!(
                        "members['{}'].hierarchy.parent ('{}') is inconsistent with \
                         topology.tree (edge declares parent '{}') — organization-packages.md \
                         'The package': a member's own hierarchy.toml must be consistent with \
                         topology.toml, lint error otherwise.",
                        member.id, declared_parent, actual_parent
                    ));
                }
                None => {
                    errors.push(format!(
                        "members['{}'].hierarchy.parent ('{}') is inconsistent with \
                         topology.tree (no edge declares a parent for '{}')",
                        member.id, declared_parent, member.id
                    ));
                }
            }
        }
        if !hierarchy.permitted_children.is_empty() {
            let declared: BTreeSet<&str> = hierarchy
                .permitted_children
                .iter()
                .map(String::as_str)
                .collect();
            let actual: BTreeSet<&str> = children_of
                .get(member.id.as_str())
                .cloned()
                .unwrap_or_default();
            if declared != actual {
                errors.push(format!(
                    "members['{}'].hierarchy.permitted_children ({declared:?}) is inconsistent \
                     with topology.tree's edges for '{}' ({actual:?})",
                    member.id, member.id
                ));
            }
        }
    }

    errors
}

/// Check 4: every `capability_mask.grant`/`deny` entry must
/// reference a capability declared in the org's own
/// `capabilities/capabilities.toml` — no masking undeclared capabilities.
fn check_capability_mask_validity(
    members: &MembersToml,
    capabilities: &CapabilitiesToml,
) -> Vec<String> {
    let mut errors = Vec::new();
    let declared: BTreeSet<&str> = capabilities
        .capability
        .iter()
        .map(|c| c.id.as_str())
        .collect();

    for member in &members.member {
        let Some(mask) = &member.capability_mask else {
            continue;
        };
        for cap in mask.grant.iter().chain(mask.deny.iter()) {
            if !declared.contains(cap.as_str()) {
                errors.push(format!(
                    "member '{}' capability_mask references '{cap}', which is not declared in \
                     capabilities/capabilities.toml — no masking undeclared capabilities",
                    member.id
                ));
            }
        }
    }

    errors
}

/// Structural check using the joined-capability rule from
/// org-package.v1's own description: "A global without a matching
/// permissions.toml policy entry fails lint"): every
/// `capabilities/capabilities.toml` entry needs a matching
/// `capabilities/permissions.toml` entry to be a real (joined) global
/// capability, and vice versa. Not one of the four required org checks,
/// but the same  precedent `check_capability_mask_validity` (check 4)
/// depends on: `declared` there is only meaningful once the global set
/// itself is join-consistent.
fn check_global_capability_join(
    capabilities: &CapabilitiesToml,
    permissions: &PermissionsToml,
) -> Vec<String> {
    let mut errors = Vec::new();
    let declared: BTreeSet<&str> = capabilities
        .capability
        .iter()
        .map(|c| c.id.as_str())
        .collect();
    let permitted: BTreeSet<&str> = permissions
        .permission
        .iter()
        .map(|p| p.capability.as_str())
        .collect();
    for cap in &declared {
        if !permitted.contains(cap) {
            errors.push(format!(
                "capability '{cap}' is declared in capabilities/capabilities.toml but has no \
                 matching entry in capabilities/permissions.toml — an entry with no permission \
                 policy is not a capability"
            ));
        }
    }
    for cap in &permitted {
        if !declared.contains(cap) {
            errors.push(format!(
                "capabilities/permissions.toml declares a policy for '{cap}', which is not \
                 defined in capabilities/capabilities.toml"
            ));
        }
    }
    errors
}

fn org_lint(path: &Path, json_output: bool) -> Result<()> {
    org_lint_at(path, &apxm_core::env::apxm_home(), json_output)
}

/// Lints an org package against an explicit `apxm_home` root. Split out
/// from [`org_lint`] so tests can point at a tempdir instead of mutating
/// the process-global `APXM_HOME` env var — same pattern as
/// `agent.rs::agent_install_to`.
fn org_lint_at(path: &Path, apxm_home: &Path, json_output: bool) -> Result<()> {
    let org = load_org(path)?;

    let mut errors = Vec::new();
    if org.org.org_id.trim().is_empty() {
        errors.push("org.toml: org_id must not be empty".to_string());
    }
    if org.org.display_name.trim().is_empty() {
        errors.push("org.toml: display_name must not be empty".to_string());
    }
    {
        let mut seen: BTreeMap<&str, usize> = BTreeMap::new();
        for member in &org.members.member {
            *seen.entry(member.id.as_str()).or_insert(0) += 1;
        }
        for (id, count) in seen {
            if count > 1 {
                errors.push(format!(
                    "agents/members.toml: member id '{id}' is declared {count} times"
                ));
            }
        }
    }

    errors.extend(check_global_capability_join(
        &org.capabilities,
        &org.permissions,
    ));
    errors.extend(check_member_resolution(&org.members, apxm_home));
    errors.extend(check_tree_well_formed(&org.topology.tree));
    errors.extend(check_hierarchy_consistency(
        &org.members,
        &org.topology.tree,
    ));
    errors.extend(check_capability_mask_validity(
        &org.members,
        &org.capabilities,
    ));

    if json_output {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "path": path.display().to_string(),
                "ok": errors.is_empty(),
                "errors": errors,
            }))?
        );
    } else {
        print_section_header("Org Lint");
        if errors.is_empty() {
            print_status_line(&org.org.org_id, Status::Ok, "no drift, schema-valid");
        } else {
            for error in &errors {
                print_status_line(&org.org.org_id, Status::Error, error);
            }
        }
    }

    if errors.is_empty() {
        Ok(())
    } else {
        Err(anyhow!(
            "{} lint error{} in {}",
            errors.len(),
            if errors.len() == 1 { "" } else { "s" },
            path.display()
        ))
    }
}

// ---------------------------------------------------------------------
// org install
// ---------------------------------------------------------------------

fn orgs_dir(apxm_home: &Path) -> PathBuf {
    apxm_home.join("orgs")
}

fn org_install(path: &Path, force: bool, json_output: bool) -> Result<()> {
    org_install_to(path, &apxm_core::env::apxm_home(), force, json_output)
}

/// Installs an org package under an explicit `apxm_home` root. Split out
/// from [`org_install`] so tests can point at a tempdir instead of
/// mutating the process-global `APXM_HOME` env var (same reasoning as
/// `agent.rs::agent_install_to`: this crate denies `unsafe_code`,
/// which `std::env::set_var` requires in Rust 2024).
fn org_install_to(path: &Path, apxm_home: &Path, force: bool, json_output: bool) -> Result<()> {
    let org = load_org(path)?;
    let dest = orgs_dir(apxm_home).join(&org.org.org_id);

    if dest.exists() {
        if !force {
            bail!(
                "'{}' already exists; pass --force to overwrite",
                dest.display()
            );
        }
        fs::remove_dir_all(&dest)
            .with_context(|| format!("Failed to remove existing {}", dest.display()))?;
    }
    fs::create_dir_all(dest.parent().expect("dest has a parent"))
        .with_context(|| format!("Failed to create {}", orgs_dir(apxm_home).display()))?;
    copy_dir_recursive(path, &dest)?;

    if json_output {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "id": org.org.org_id,
                "installed_to": dest.display().to_string(),
            }))?
        );
    } else {
        print_section_header("Org Installed");
        print_status_line(&org.org.org_id, Status::Ok, &dest.display().to_string());
    }
    let _ = &org.root; // root retained on LoadedOrg for parity/debuggability
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::agent::{agent_install_to, agent_new};
    use tempfile::tempdir;

    fn scaffold(dir: &Path, id: &str) {
        org_new(id, Some(dir.to_path_buf()), None, true).expect("scaffold ok");
    }

    /// Scaffold + install a minimal agent under `apxm_home`
    /// so org-level member-resolution tests have something real to resolve
    /// against, at the given version.
    fn install_agent_fixture(apxm_home: &Path, id: &str, version: &str) {
        let tmp = tempdir().unwrap();
        let root = tmp.path().join(id);
        agent_new(id, Some(root.clone()), None, "looped-agent", true).expect("agent scaffold ok");
        let agent_toml = fs::read_to_string(root.join("agent.toml")).unwrap();
        fs::write(
            root.join("agent.toml"),
            agent_toml.replace("version = \"0.1.0\"", &format!("version = \"{version}\"")),
        )
        .unwrap();
        agent_install_to(&root, apxm_home, false, true).expect("agent install ok");
    }

    #[test]
    fn new_scaffolds_schema_valid_tree() {
        let tmp = tempdir().unwrap();
        let root = tmp.path().join("demo-org");
        scaffold(&root, "demo-org");

        for rel in [
            "org.toml",
            "capabilities/capabilities.toml",
            "capabilities/permissions.toml",
            "agents/members.toml",
            "topology.toml",
            "prompts/persona.md",
            "tests/README.md",
        ] {
            assert!(root.join(rel).is_file(), "missing scaffolded file: {rel}");
        }

        let org: OrgToml = read_toml(&root.join("org.toml")).unwrap();
        assert_eq!(org.org_id, "demo-org");

        let topology: TopologyToml = read_toml(&root.join("topology.toml")).unwrap();
        assert_eq!(topology.tree.root, "demo-org-root");
        assert!(topology.tree.edges.is_empty());

        let members: MembersToml = read_toml(&root.join("agents/members.toml")).unwrap();
        assert!(members.member.is_empty());

        // The freshly scaffolded tree (no members yet) must lint clean.
        let fake_home = tempdir().unwrap();
        org_lint_at(&root, fake_home.path(), true).expect("scaffolded org should lint clean");
    }

    #[test]
    fn new_refuses_nonempty_destination() {
        let tmp = tempdir().unwrap();
        let root = tmp.path().join("demo-org");
        scaffold(&root, "demo-org");
        let err = org_new("demo-org", Some(root.clone()), None, true).unwrap_err();
        assert!(err.to_string().contains("already exists"));
    }

    /// Builds a valid two-member org package (root -> child), using the
    ///  vector `valid-two-level-tree-with-global-capability-mask-and-delegation`.
    fn build_valid_org(root: &Path, apxm_home: &Path) {
        scaffold(root, "valid-org");
        install_agent_fixture(apxm_home, "root-pkg", "0.1.0");
        install_agent_fixture(apxm_home, "child-pkg", "0.2.0");

        fs::write(
            root.join("capabilities/capabilities.toml"),
            "[[capability]]\nid = \"org.http_get\"\ndescription = \"demo\"\n",
        )
        .unwrap();
        fs::write(
            root.join("capabilities/permissions.toml"),
            "[[permission]]\ncapability = \"org.http_get\"\ndecision = \"allow\"\n",
        )
        .unwrap();
        fs::write(
            root.join("agents/members.toml"),
            "[[member]]\n\
             id = \"root-agent\"\n\
             package = \"root-pkg\"\n\
             version = \"0.1.0\"\n\
             \n\
             [member.hierarchy]\n\
             permitted_children = [\"child-agent\"]\n\
             \n\
             [[member]]\n\
             id = \"child-agent\"\n\
             package = \"child-pkg\"\n\
             version = \"0.2.0\"\n\
             \n\
             [member.capability_mask]\n\
             deny = [\"org.http_get\"]\n\
             \n\
             [member.hierarchy]\n\
             parent = \"root-agent\"\n",
        )
        .unwrap();
        fs::write(
            root.join("topology.toml"),
            "[tree]\n\
             root = \"root-agent\"\n\
             edges = [{ parent = \"root-agent\", child = \"child-agent\" }]\n\
             \n\
             [[relations]]\n\
             from = \"root-agent\"\n\
             to = \"child-agent\"\n\
             type = \"delegates_to\"\n",
        )
        .unwrap();
    }

    #[test]
    fn lint_passes_on_consistent_org() {
        let tmp = tempdir().unwrap();
        let root = tmp.path().join("valid-org");
        let fake_home = tempdir().unwrap();
        build_valid_org(&root, fake_home.path());

        org_lint_at(&root, fake_home.path(), true).expect("consistent org should lint clean");
    }

    #[test]
    fn lint_catches_unresolvable_member() {
        let tmp = tempdir().unwrap();
        let root = tmp.path().join("valid-org");
        let fake_home = tempdir().unwrap();
        build_valid_org(&root, fake_home.path());

        // Point a member at a package that was never installed.
        let members_toml = fs::read_to_string(root.join("agents/members.toml")).unwrap();
        fs::write(
            root.join("agents/members.toml"),
            members_toml.replace("package = \"child-pkg\"", "package = \"nowhere-pkg\""),
        )
        .unwrap();

        let err = org_lint_at(&root, fake_home.path(), true)
            .expect_err("unresolvable member must fail lint");
        assert!(err.to_string().contains("lint error"));
    }

    #[test]
    fn lint_catches_unsatisfied_version_requirement() {
        let tmp = tempdir().unwrap();
        let root = tmp.path().join("valid-org");
        let fake_home = tempdir().unwrap();
        build_valid_org(&root, fake_home.path());

        // Require a version the installed agent does not satisfy.
        let members_toml = fs::read_to_string(root.join("agents/members.toml")).unwrap();
        fs::write(
            root.join("agents/members.toml"),
            members_toml.replace(
                "package = \"child-pkg\"\nversion = \"0.2.0\"",
                "package = \"child-pkg\"\nversion = \"9.9.9\"",
            ),
        )
        .unwrap();

        let err = org_lint_at(&root, fake_home.path(), true)
            .expect_err("unsatisfied version requirement must fail lint");
        assert!(err.to_string().contains("lint error"));
    }

    #[test]
    fn lint_catches_cyclic_tree() {
        let tmp = tempdir().unwrap();
        let root = tmp.path().join("valid-org");
        let fake_home = tempdir().unwrap();
        build_valid_org(&root, fake_home.path());

        // root -> child -> root: a cycle back to the declared root.
        fs::write(
            root.join("topology.toml"),
            "[tree]\n\
             root = \"root-agent\"\n\
             edges = [\n\
             \x20\x20{ parent = \"root-agent\", child = \"child-agent\" },\n\
             \x20\x20{ parent = \"child-agent\", child = \"root-agent\" },\n\
             ]\n",
        )
        .unwrap();
        // Drop the hierarchy snapshots so this test isolates the tree
        // well-formedness check from the hierarchy-consistency check.
        fs::write(
            root.join("agents/members.toml"),
            "[[member]]\nid = \"root-agent\"\npackage = \"root-pkg\"\nversion = \"0.1.0\"\n\n\
             [[member]]\nid = \"child-agent\"\npackage = \"child-pkg\"\nversion = \"0.2.0\"\n",
        )
        .unwrap();

        let err =
            org_lint_at(&root, fake_home.path(), true).expect_err("cyclic tree must fail lint");
        assert!(err.to_string().contains("lint error"));
    }

    #[test]
    fn lint_catches_malformed_multi_root_tree() {
        let tmp = tempdir().unwrap();
        let root = tmp.path().join("valid-org");
        let fake_home = tempdir().unwrap();
        build_valid_org(&root, fake_home.path());

        // child-agent has no parent edge at all -> two roots, not a tree.
        fs::write(
            root.join("topology.toml"),
            "[tree]\n\
             root = \"root-agent\"\n\
             edges = []\n",
        )
        .unwrap();
        fs::write(
            root.join("agents/members.toml"),
            "[[member]]\nid = \"root-agent\"\npackage = \"root-pkg\"\nversion = \"0.1.0\"\n\n\
             [[member]]\nid = \"child-agent\"\npackage = \"child-pkg\"\nversion = \"0.2.0\"\n",
        )
        .unwrap();

        // This tree is trivially well-formed on its own (a single node
        // `root-agent`, no edges) — the malformation here is instead
        // exercised directly against the tree-check function with an
        // explicit two-root edge set, since `child-agent` floating with no
        // edges at all is indistinguishable from "not part of the tree".
        let bad_tree = TreeToml {
            root: "root-agent".to_string(),
            edges: vec![
                EdgeToml {
                    parent: "root-agent".to_string(),
                    child: "a".to_string(),
                },
                EdgeToml {
                    parent: "other-root".to_string(),
                    child: "b".to_string(),
                },
            ],
        };
        let errors = check_tree_well_formed(&bad_tree);
        assert!(
            errors
                .iter()
                .any(|e| e.contains("expected exactly one root")),
            "expected a multi-root error, got: {errors:?}"
        );
    }

    #[test]
    fn lint_catches_hierarchy_contradicting_topology() {
        // Uses the `invalid-member-hierarchy-contradicts-topology-tree`
        // vector: child-agent's own hierarchy.parent says 'other-agent', but
        // topology.tree declares its parent as 'root-agent'.
        let tmp = tempdir().unwrap();
        let root = tmp.path().join("bad-hierarchy-org");
        let fake_home = tempdir().unwrap();
        scaffold(&root, "bad-hierarchy-org");
        install_agent_fixture(fake_home.path(), "root-pkg", "0.1.0");
        install_agent_fixture(fake_home.path(), "child-pkg", "0.2.0");
        install_agent_fixture(fake_home.path(), "other-pkg", "0.1.0");

        fs::write(
            root.join("agents/members.toml"),
            "[[member]]\n\
             id = \"root-agent\"\n\
             package = \"root-pkg\"\n\
             version = \"0.1.0\"\n\
             \n\
             [member.hierarchy]\n\
             permitted_children = [\"child-agent\"]\n\
             \n\
             [[member]]\n\
             id = \"child-agent\"\n\
             package = \"child-pkg\"\n\
             version = \"0.2.0\"\n\
             \n\
             [member.hierarchy]\n\
             parent = \"other-agent\"\n\
             \n\
             [[member]]\n\
             id = \"other-agent\"\n\
             package = \"other-pkg\"\n\
             version = \"0.1.0\"\n",
        )
        .unwrap();
        fs::write(
            root.join("topology.toml"),
            "[tree]\n\
             root = \"root-agent\"\n\
             edges = [\n\
             \x20\x20{ parent = \"root-agent\", child = \"child-agent\" },\n\
             \x20\x20{ parent = \"root-agent\", child = \"other-agent\" },\n\
             ]\n",
        )
        .unwrap();

        let err = org_lint_at(&root, fake_home.path(), true)
            .expect_err("hierarchy/topology contradiction must fail lint");
        assert!(
            err.to_string().contains("lint error"),
            "expected a lint error, got: {err}"
        );

        // Re-run just the targeted check to assert on the exact message
        // shape from the vector's `expected_error`.
        let org = load_org(&root).unwrap();
        let errors = check_hierarchy_consistency(&org.members, &org.topology.tree);
        assert!(
            errors.iter().any(|e| {
                e.contains("members['child-agent'].hierarchy.parent ('other-agent')")
                    && e.contains("edge declares parent 'root-agent'")
            }),
            "expected the hierarchy-vs-topology contradiction message, got: {errors:?}"
        );
    }

    #[test]
    fn lint_catches_invalid_capability_mask() {
        let tmp = tempdir().unwrap();
        let root = tmp.path().join("valid-org");
        let fake_home = tempdir().unwrap();
        build_valid_org(&root, fake_home.path());

        // Deny a capability that was never declared in capabilities.toml.
        let members_toml = fs::read_to_string(root.join("agents/members.toml")).unwrap();
        fs::write(
            root.join("agents/members.toml"),
            members_toml.replace(
                "deny = [\"org.http_get\"]",
                "deny = [\"org.http_get\", \"org.undeclared\"]",
            ),
        )
        .unwrap();

        let err = org_lint_at(&root, fake_home.path(), true)
            .expect_err("undeclared capability mask entry must fail lint");
        assert!(err.to_string().contains("lint error"));
    }

    #[test]
    fn install_places_files_under_apxm_home() {
        let tmp = tempdir().unwrap();
        let root = tmp.path().join("installable-org");
        scaffold(&root, "installable-org");
        let fake_home = tempdir().unwrap();

        org_install_to(&root, fake_home.path(), false, true).expect("install ok");

        let dest = fake_home.path().join("orgs").join("installable-org");
        assert!(dest.join("org.toml").is_file());
        assert!(dest.join("agents/members.toml").is_file());
        assert!(dest.join("topology.toml").is_file());

        let real_home = dirs::home_dir().unwrap_or_default();
        assert!(!real_home.join(".apxm/orgs/installable-org").exists());
    }

    #[test]
    fn install_refuses_existing_destination_without_force() {
        let tmp = tempdir().unwrap();
        let root = tmp.path().join("dupe-org");
        scaffold(&root, "dupe-org");
        let fake_home = tempdir().unwrap();

        org_install_to(&root, fake_home.path(), false, true).expect("first install ok");
        let second = org_install_to(&root, fake_home.path(), false, true);
        let err = second.expect_err("second install without --force must fail");
        assert!(err.to_string().contains("already exists"));
    }
}
