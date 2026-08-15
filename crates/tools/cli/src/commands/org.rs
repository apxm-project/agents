//! `apxm org new|lint|install` — the toolchain for the canonical
//! organization-package folder format (`apxm.org`).
//!
//! An org package is one authored file: `org.toml`. The org's global
//! Capability set is the key set of `[permissions]`: a capability id with a
//! decision is the declaration, so there is no second inventory for it to
//! agree with. Members and topology are tables of the same manifest.
//!
//! `[[members]]` entries carry a `hierarchy` snapshot of the referenced
//! agent's own `agent.toml [hierarchy]`. That snapshot — not a second
//! resolve-and-read of the installed agent's manifest — is what `org lint`
//! compares against `[topology.tree]`.
//!
//! The recognized folder contract is derived from
//! `contracts/schemas/apxm.org.json`'s `PackageFiles.patternProperties`.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use anyhow::{Context, Result, anyhow, bail};
use apxm_ais::permissions::PermissionDecision;
use regex::Regex;
use serde::{Deserialize, Serialize};

use super::agent::{
    HierarchyToml, copy_dir_recursive, package_file_patterns, resolve_installed_agent,
    unrecognized_files_under,
};
use super::implementations::{Status, print_section_header, print_status_line};

const ORG_SCHEMA: &str = "apxm.org";

/// The published `apxm.org` contract, embedded from its checked-in bytes.
pub(crate) const ORG_SCHEMA_JSON: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../../contracts/schemas/apxm.org.json"
));

fn recognized_relpath_patterns() -> &'static [Regex] {
    static PATTERNS: OnceLock<Vec<Regex>> = OnceLock::new();
    PATTERNS.get_or_init(|| package_file_patterns(ORG_SCHEMA_JSON, ORG_SCHEMA))
}

#[cfg(test)]
fn recognized_relpath(rel: &str) -> bool {
    recognized_relpath_patterns()
        .iter()
        .any(|pattern| pattern.is_match(rel))
}

fn find_unrecognized_files(root: &Path) -> Result<Vec<String>> {
    unrecognized_files_under(root, recognized_relpath_patterns())
}

// ---------------------------------------------------------------------
// On-disk manifest shapes (`apxm.org` projections)
// ---------------------------------------------------------------------

/// Projection of `org.toml`. Scalars first, tables last so a round-trip
/// through `toml::to_string` emits valid TOML. `deny_unknown_fields` makes a
/// retired key (`capability`, a leftover inventory) a parse error.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OrgToml {
    pub org_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub schema_version: Option<String>,
    pub display_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub policy: Option<toml::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub environment: Option<toml::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub models: Option<toml::Value>,
    /// The org's global Capability set. A key with a decision is the
    /// declaration — there is no second inventory for it to agree with.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub permissions: BTreeMap<String, PermissionDecision>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub members: Vec<MemberEntry>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub topology: Option<TopologyToml>,
}

/// The org-global capability set, read for `agent lint --org` so a member
/// package's `agent.toml [permissions]` may state a decision for a capability
/// the org supplies.
pub(super) fn load_org_global_capabilities(org_root: &Path) -> Result<BTreeSet<String>> {
    Ok(load_org(org_root)?.org.permissions.into_keys().collect())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
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
#[serde(deny_unknown_fields)]
pub struct CapabilityMaskToml {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub grant: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub deny: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
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
#[serde(deny_unknown_fields)]
pub struct TreeToml {
    pub root: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub edges: Vec<EdgeToml>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EdgeToml {
    pub parent: String,
    pub child: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
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

    let org_toml = format!(
        "org_id = \"{id}\"\n\
         schema_version = \"{ORG_SCHEMA}\"\n\
         display_name = \"{display_name}\"\n\
         description = \"Describe this org's purpose here.\"\n\n\
         [policy]\n\
         approval_default = \"manual\"\n\
         autonomy_ceiling = \"supervised\"\n\n\
         # Global Capability set: a key with a decision is the declaration.\n\
         # Dotted ids must be quoted: \"org.http_get\" = \"allow\"\n\
         # [permissions]\n\n\
         # [[members]]\n\
         # id = \"root-agent\"\n\
         # package = \"some.agent\"\n\
         # version = \"0.1.0\"\n\n\
         [topology.tree]\n\
         root = \"{placeholder_root_member}\"\n\
         edges = []\n"
    );
    write_new_file(&root.join("org.toml"), &org_toml)?;
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
    Ok(LoadedOrg {
        root: root.to_path_buf(),
        org,
    })
}

// ---------------------------------------------------------------------
// org lint
// ---------------------------------------------------------------------

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

fn check_member_resolution(members: &[MemberEntry], apxm_home: &Path) -> Vec<String> {
    let mut errors = Vec::new();
    for member in members {
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

fn check_hierarchy_consistency(members: &[MemberEntry], tree: &TreeToml) -> Vec<String> {
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

    for member in members {
        let Some(hierarchy) = &member.hierarchy else {
            continue;
        };
        if let Some(declared_parent) = &hierarchy.parent {
            match parent_of.get(member.id.as_str()) {
                Some(actual_parent) if actual_parent == declared_parent => {}
                Some(actual_parent) => {
                    errors.push(format!(
                        "members['{}'].hierarchy.parent ('{}') is inconsistent with \
                         topology.tree (edge declares parent '{}') — a member's own \
                         agent.toml [hierarchy] must be consistent with org.toml [topology.tree]",
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

fn check_capability_mask_validity(
    members: &[MemberEntry],
    declared: &BTreeSet<String>,
) -> Vec<String> {
    let mut errors = Vec::new();
    for member in members {
        let Some(mask) = &member.capability_mask else {
            continue;
        };
        for cap in mask.grant.iter().chain(mask.deny.iter()) {
            if !declared.contains(cap) {
                errors.push(format!(
                    "member '{}' capability_mask references '{cap}', which is not a key of \
                     org.toml [permissions] — no masking undeclared capabilities",
                    member.id
                ));
            }
        }
    }
    errors
}

fn org_lint(path: &Path, json_output: bool) -> Result<()> {
    org_lint_at(path, &apxm_core::env::apxm_home(), json_output)
}

fn org_lint_at(path: &Path, apxm_home: &Path, json_output: bool) -> Result<()> {
    let org = load_org(path)?;

    let mut errors = Vec::new();
    if org.org.org_id.trim().is_empty() {
        errors.push("org.toml: org_id must not be empty".to_string());
    }
    if org.org.display_name.trim().is_empty() {
        errors.push("org.toml: display_name must not be empty".to_string());
    }
    match org.org.schema_version.as_deref() {
        Some(ORG_SCHEMA) => {}
        Some(other) => errors.push(format!(
            "org.toml: schema_version '{other}' must be '{ORG_SCHEMA}'"
        )),
        None => errors.push(format!(
            "org.toml: schema_version is required and must be '{ORG_SCHEMA}'"
        )),
    }
    {
        let mut seen: BTreeMap<&str, usize> = BTreeMap::new();
        for member in &org.org.members {
            *seen.entry(member.id.as_str()).or_insert(0) += 1;
        }
        for (id, count) in seen {
            if count > 1 {
                errors.push(format!(
                    "org.toml: member id '{id}' is declared {count} times"
                ));
            }
        }
    }

    errors.extend(check_member_resolution(&org.org.members, apxm_home));
    if let Some(topology) = &org.org.topology {
        errors.extend(check_tree_well_formed(&topology.tree));
        errors.extend(check_hierarchy_consistency(
            &org.org.members,
            &topology.tree,
        ));
    }
    let declared: BTreeSet<String> = org.org.permissions.keys().cloned().collect();
    errors.extend(check_capability_mask_validity(&org.org.members, &declared));
    for unrecognized in find_unrecognized_files(path)? {
        errors.push(format!(
            "unrecognized file '{unrecognized}' is not part of the {ORG_SCHEMA} folder contract"
        ));
    }

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
    let _ = &org.root;
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

    fn write_org_toml(root: &Path, contents: &str) {
        fs::create_dir_all(root).unwrap();
        fs::write(root.join("org.toml"), contents).unwrap();
    }

    fn two_member_org_toml(child_package: &str, child_version: &str, deny: &str) -> String {
        format!(
            "org_id = \"valid-org\"\n\
             schema_version = \"apxm.org\"\n\
             display_name = \"Valid Org\"\n\n\
             [permissions]\n\
             \"org.http_get\" = \"allow\"\n\n\
             [[members]]\n\
             id = \"root-agent\"\n\
             package = \"root-pkg\"\n\
             version = \"0.1.0\"\n\
             [members.hierarchy]\n\
             permitted_children = [\"child-agent\"]\n\n\
             [[members]]\n\
             id = \"child-agent\"\n\
             package = \"{child_package}\"\n\
             version = \"{child_version}\"\n\
             [members.capability_mask]\n\
             deny = [{deny}]\n\
             [members.hierarchy]\n\
             parent = \"root-agent\"\n\n\
             [topology.tree]\n\
             root = \"root-agent\"\n\
             edges = [{{ parent = \"root-agent\", child = \"child-agent\" }}]\n\n\
             [[topology.relations]]\n\
             from = \"root-agent\"\n\
             to = \"child-agent\"\n\
             type = \"delegates_to\"\n"
        )
    }

    fn build_valid_org(root: &Path, apxm_home: &Path) {
        install_agent_fixture(apxm_home, "root-pkg", "0.1.0");
        install_agent_fixture(apxm_home, "child-pkg", "0.2.0");
        write_org_toml(
            root,
            &two_member_org_toml("child-pkg", "0.2.0", "\"org.http_get\""),
        );
    }

    fn fixture_version(requirement: &str) -> &str {
        requirement
            .strip_prefix('^')
            .or_else(|| requirement.strip_prefix('~'))
            .unwrap_or(requirement)
    }

    fn org_vectors() -> Vec<serde_json::Value> {
        let text = include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../../contracts/vectors/apxm.org.json"
        ));
        serde_json::from_str::<Vec<serde_json::Value>>(text)
            .expect("the published apxm.org vectors are a JSON array")
    }

    fn manifest_toml(org: &serde_json::Value) -> Result<String, String> {
        let from_json = toml::to_string(org).map_err(|error| error.to_string())?;
        if toml::from_str::<OrgToml>(&from_json).is_ok() {
            return Ok(from_json);
        }
        let typed: OrgToml =
            serde_json::from_value(org.clone()).map_err(|error| error.to_string())?;
        toml::to_string(&typed).map_err(|error| error.to_string())
    }

    fn lint_admits_vector(vector: &serde_json::Value) -> Result<(), String> {
        let files = vector["files"].as_object().expect("vector files map");
        for digest in files.values() {
            let digest = digest.as_str().ok_or("a file digest must be a string")?;
            if digest.len() != 64
                || !digest
                    .bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
            {
                return Err(format!(
                    "file digest {digest:?} is not lowercase sha256 hex"
                ));
            }
        }

        let tmp = tempdir().map_err(|error| error.to_string())?;
        let root = tmp.path().join("package");
        for path in files.keys() {
            let file = root.join(path);
            if let Some(parent) = file.parent() {
                fs::create_dir_all(parent).map_err(|error| error.to_string())?;
            }
            fs::write(&file, "").map_err(|error| error.to_string())?;
        }
        let manifest = manifest_toml(&vector["org"])?;
        fs::write(root.join("org.toml"), manifest).map_err(|error| error.to_string())?;

        let fake_home = tempdir().map_err(|error| error.to_string())?;
        if let Some(members) = vector["org"]["members"].as_array() {
            let mut installed: BTreeSet<(String, String)> = BTreeSet::new();
            for member in members {
                let Some(package) = member["package"].as_str() else {
                    continue;
                };
                let Some(version) = member["version"].as_str() else {
                    continue;
                };
                let install_version = fixture_version(version).to_string();
                if installed.insert((package.to_string(), install_version.clone())) {
                    install_agent_fixture(fake_home.path(), package, &install_version);
                }
            }
        }

        org_lint_at(&root, fake_home.path(), true).map_err(|error| error.to_string())
    }

    #[test]
    fn new_scaffolds_schema_valid_tree() {
        let tmp = tempdir().unwrap();
        let root = tmp.path().join("demo-org");
        scaffold(&root, "demo-org");

        for rel in ["org.toml", "prompts/persona.md", "tests/README.md"] {
            assert!(root.join(rel).is_file(), "missing scaffolded file: {rel}");
        }
        for retired in [
            "capabilities/capabilities.toml",
            "capabilities/permissions.toml",
            "agents/members.toml",
            "topology.toml",
        ] {
            assert!(
                !root.join(retired).exists(),
                "scaffold must not write retired path {retired}"
            );
        }

        let org: OrgToml = read_toml(&root.join("org.toml")).unwrap();
        assert_eq!(org.org_id, "demo-org");
        assert_eq!(org.schema_version.as_deref(), Some(ORG_SCHEMA));
        assert!(org.members.is_empty());
        assert_eq!(
            org.topology.as_ref().map(|t| t.tree.root.as_str()),
            Some("demo-org-root")
        );

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
        install_agent_fixture(fake_home.path(), "root-pkg", "0.1.0");
        write_org_toml(
            &root,
            &two_member_org_toml("nowhere-pkg", "0.2.0", "\"org.http_get\""),
        );

        let err = org_lint_at(&root, fake_home.path(), true)
            .expect_err("unresolvable member must fail lint");
        assert!(err.to_string().contains("lint error"));
    }

    #[test]
    fn lint_catches_unsatisfied_version_requirement() {
        let tmp = tempdir().unwrap();
        let root = tmp.path().join("valid-org");
        let fake_home = tempdir().unwrap();
        install_agent_fixture(fake_home.path(), "root-pkg", "0.1.0");
        install_agent_fixture(fake_home.path(), "child-pkg", "0.2.0");
        write_org_toml(
            &root,
            &two_member_org_toml("child-pkg", "9.9.9", "\"org.http_get\""),
        );

        let err = org_lint_at(&root, fake_home.path(), true)
            .expect_err("unsatisfied version requirement must fail lint");
        assert!(err.to_string().contains("lint error"));
    }

    #[test]
    fn lint_catches_cyclic_tree() {
        let tmp = tempdir().unwrap();
        let root = tmp.path().join("valid-org");
        let fake_home = tempdir().unwrap();
        install_agent_fixture(fake_home.path(), "root-pkg", "0.1.0");
        install_agent_fixture(fake_home.path(), "child-pkg", "0.2.0");
        write_org_toml(
            &root,
            "org_id = \"valid-org\"\n\
             schema_version = \"apxm.org\"\n\
             display_name = \"Valid Org\"\n\n\
             [[members]]\n\
             id = \"root-agent\"\n\
             package = \"root-pkg\"\n\
             version = \"0.1.0\"\n\n\
             [[members]]\n\
             id = \"child-agent\"\n\
             package = \"child-pkg\"\n\
             version = \"0.2.0\"\n\n\
             [topology.tree]\n\
             root = \"root-agent\"\n\
             edges = [\n\
             \x20\x20{ parent = \"root-agent\", child = \"child-agent\" },\n\
             \x20\x20{ parent = \"child-agent\", child = \"root-agent\" },\n\
             ]\n",
        );

        let err =
            org_lint_at(&root, fake_home.path(), true).expect_err("cyclic tree must fail lint");
        assert!(err.to_string().contains("lint error"));
    }

    #[test]
    fn lint_catches_malformed_multi_root_tree() {
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
        let tmp = tempdir().unwrap();
        let root = tmp.path().join("bad-hierarchy-org");
        let fake_home = tempdir().unwrap();
        install_agent_fixture(fake_home.path(), "root-pkg", "0.1.0");
        install_agent_fixture(fake_home.path(), "child-pkg", "0.2.0");
        install_agent_fixture(fake_home.path(), "other-pkg", "0.1.0");
        write_org_toml(
            &root,
            "org_id = \"bad-hierarchy-org\"\n\
             schema_version = \"apxm.org\"\n\
             display_name = \"Bad Hierarchy\"\n\n\
             [[members]]\n\
             id = \"root-agent\"\n\
             package = \"root-pkg\"\n\
             version = \"0.1.0\"\n\
             [members.hierarchy]\n\
             permitted_children = [\"child-agent\"]\n\n\
             [[members]]\n\
             id = \"child-agent\"\n\
             package = \"child-pkg\"\n\
             version = \"0.2.0\"\n\
             [members.hierarchy]\n\
             parent = \"other-agent\"\n\n\
             [[members]]\n\
             id = \"other-agent\"\n\
             package = \"other-pkg\"\n\
             version = \"0.1.0\"\n\n\
             [topology.tree]\n\
             root = \"root-agent\"\n\
             edges = [\n\
             \x20\x20{ parent = \"root-agent\", child = \"child-agent\" },\n\
             \x20\x20{ parent = \"root-agent\", child = \"other-agent\" },\n\
             ]\n",
        );

        let err = org_lint_at(&root, fake_home.path(), true)
            .expect_err("hierarchy/topology contradiction must fail lint");
        assert!(
            err.to_string().contains("lint error"),
            "expected a lint error, got: {err}"
        );

        let org = load_org(&root).unwrap();
        let tree = &org.org.topology.as_ref().unwrap().tree;
        let errors = check_hierarchy_consistency(&org.org.members, tree);
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
        install_agent_fixture(fake_home.path(), "root-pkg", "0.1.0");
        install_agent_fixture(fake_home.path(), "child-pkg", "0.2.0");
        write_org_toml(
            &root,
            &two_member_org_toml("child-pkg", "0.2.0", "\"org.http_get\", \"org.undeclared\""),
        );

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
        assert!(dest.join("prompts/persona.md").is_file());
        assert!(!dest.join("agents/members.toml").exists());
        assert!(!dest.join("topology.toml").exists());

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

    #[test]
    fn org_vectors_match_the_lint_path() {
        for vector in org_vectors() {
            let name = vector["name"].as_str().expect("vector name");
            let expected = vector["expected_valid"].as_bool().expect("expected_valid");
            let verdict = lint_admits_vector(&vector["input"]);
            assert_eq!(
                verdict.is_ok(),
                expected,
                "vector '{name}' expected valid={expected} but org lint returned {verdict:?}",
            );
        }
    }

    #[test]
    fn the_recognized_folder_contract_is_the_published_one() {
        for recognized in [
            "org.toml",
            "README.md",
            "prompts/persona.md",
            "tests/README.md",
        ] {
            assert!(
                recognized_relpath(recognized),
                "the published contract must recognize {recognized}"
            );
        }
        for retired in [
            "capabilities/capabilities.toml",
            "capabilities/permissions.toml",
            "agents/members.toml",
            "topology.toml",
            "notes.txt",
        ] {
            assert!(
                !recognized_relpath(retired),
                "the published contract must not recognize {retired}"
            );
        }
    }

    #[test]
    fn the_schema_version_constant_is_read_from_the_published_contract() {
        let schema: serde_json::Value =
            serde_json::from_str(ORG_SCHEMA_JSON).expect("the embedded contract is valid JSON");
        assert_eq!(schema["$id"].as_str(), Some(ORG_SCHEMA));
        assert_eq!(
            schema["$defs"]["OrgManifest"]["properties"]["schema_version"]["const"].as_str(),
            Some(ORG_SCHEMA),
            "the schema_version constant drifted from the schema `const`",
        );
    }

    #[test]
    fn checked_in_packages_satisfy_the_published_folder_contract() {
        let repository_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..");
        let package = "examples/orgs/demo";
        let root = repository_root.join(package);
        let unrecognized = find_unrecognized_files(&root)
            .unwrap_or_else(|error| panic!("walk {package}: {error}"));
        assert!(
            unrecognized.is_empty(),
            "{package} carries paths outside the published folder contract: {unrecognized:?}"
        );
        let pkg = load_org(&root).unwrap_or_else(|error| panic!("load {package}: {error}"));
        assert_eq!(pkg.org.schema_version.as_deref(), Some(ORG_SCHEMA));
        assert!(pkg.org.members.is_empty());
        org_lint_at(&root, tempdir().unwrap().path(), true)
            .unwrap_or_else(|error| panic!("lint {package}: {error}"));
    }
}
