//! `apxm package new|lint|build|install` — the AGT-2 toolchain for the
//! canonical agent-package folder format (AGT-1, `apxm.agent-package.v1`,
//! `workspace/contracts/schemas/agent-package.v1.json`).
//!
//! The folder contract, required/optional files, and integrity hash-chain
//! algorithm are hand-ported from that schema into the Rust checks below
//! (the same pattern `ApxmPaths::discover` uses for `apxm.state-layout.v1`
//! in `crates/machine/contracts/src/paths.rs`) rather than loading the JSON
//! schema file at runtime — this repo does not vendor or path-depend on the
//! sibling `contracts` repo.
//!
//! Capability-set drift (agent.toml vs capabilities/capabilities.toml vs
//! capabilities/permissions.toml vs skills/) is enforced as a lint ERROR
//! per the plan's non-negotiable. No hand-rolled equivalent of this check
//! was found elsewhere in the workspace at the AGT-1 schema's grammar (the
//! only existing "undeclared capability" check is
//! `server/crates/core/src/skills.rs::validate_inv_cap_node`, which compares
//! a *compiled skill artifact's* INV_CAP nodes against a manifest's
//! `declared_tools` set — a different, narrower check over compiled AIR, not
//! over the package's authored `capabilities.toml`/`permissions.toml`/
//! `agent.toml` triple). This module's drift check is therefore a fresh
//! implementation of the AGT-1 "joined capability" semantics described in
//! `docs/plans/agent-package-format.md`, not a port of that server check.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow, bail};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::implementations::{Status, print_section_header, print_status_line};

// ---------------------------------------------------------------------
// On-disk manifest shapes (agent-package.v1 projections)
// ---------------------------------------------------------------------

/// Projection of `pack.toml` (`agent-package.v1#/properties/pack`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PackToml {
    pub pack_id: String,
    pub version: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub license: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<toml::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub compile: Option<toml::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub integrity: Option<IntegrityToml>,
}

/// Mirrors `agent-package.v1#/properties/integrity`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IntegrityToml {
    pub algorithm: String,
    pub pack_hash: String,
    pub chain: Vec<ChainLinkToml>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChainLinkToml {
    pub path: String,
    pub prev_hash: String,
    pub hash: String,
}

/// Projection of `agent.toml` (`agent-package.v1#/properties/agent`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentToml {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub domain: Option<String>,
    /// Path relative to `python/` to the Python-frontend entry.
    pub entry: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runtime: Option<toml::Value>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub prompts: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub capabilities: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub skills: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chat: Option<toml::Value>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub hooks: Vec<HookToml>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HookToml {
    pub event: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#match: Option<String>,
    pub mode: String,
    pub handler: String,
}

/// Projection of the optional `hierarchy.toml`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct HierarchyToml {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub permitted_children: Vec<String>,
}

/// `capabilities/capabilities.toml` — array of capability definitions. This
/// grammar is not yet standardized by a published `capability-definition.v1`
/// schema in `contracts/schemas/` (CM-1 is still pending); the shape below
/// follows the one worked example in the workspace
/// (`studio/agents/gao/capabilities/capabilities.toml`, pre-canonical dialect)
/// simplified to what AGT-1's lint needs: a stable `id` per capability.
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

/// `capabilities/permissions.toml` — one policy entry per joined capability
/// id (the AGT-1 "joined capability" rule: every `capabilities.toml` entry
/// must have a matching permission policy here or it is not a capability).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PermissionsToml {
    #[serde(default, rename = "permission")]
    pub permission: Vec<PermissionEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PermissionEntry {
    pub capability: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub decision: Option<String>,
    #[serde(flatten)]
    pub extra: toml::Table,
}

/// `skills/<id>/skill.toml` — always present per skill (AGT-1).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillToml {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// `false` (default) => prompt-only (`CompileStatus::NotCompiled`).
    #[serde(default)]
    pub compiled: bool,
    /// Capability ids this skill invokes; each must be in the package's
    /// joined capability set (AGT-5).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub capabilities: Vec<String>,
}

// ---------------------------------------------------------------------
// package new
// ---------------------------------------------------------------------

pub fn package_command(action: super::PackageAction, json_output: bool) -> Result<()> {
    match action {
        super::PackageAction::New {
            id,
            path,
            display_name,
        } => package_new(&id, path, display_name, json_output),
        super::PackageAction::Lint { path } => package_lint(&path, json_output),
        super::PackageAction::Build { path } => package_build(&path, json_output),
        super::PackageAction::Install { path, force } => {
            package_install(&path, force, json_output)
        }
    }
}

fn default_package_root(id: &str) -> PathBuf {
    PathBuf::from("packages").join(id)
}

fn titleize(id: &str) -> String {
    id.split(|c: char| c == '-' || c == '_')
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

pub(crate) fn package_new(
    id: &str,
    path: Option<PathBuf>,
    display_name: Option<String>,
    json_output: bool,
) -> Result<()> {
    if id.trim().is_empty() {
        bail!("package id must not be empty");
    }
    let root = path.unwrap_or_else(|| default_package_root(id));
    if root.exists() && fs::read_dir(&root)?.next().is_some() {
        bail!(
            "destination '{}' already exists and is not empty",
            root.display()
        );
    }

    let display_name = display_name.unwrap_or_else(|| titleize(id));
    let template_skill_id = format!("{id}-skill");

    // pack.toml
    let pack_toml = format!(
        "pack_id = \"{id}\"\nversion = \"0.1.0\"\nlicense = \"MIT\"\n\n\
         [source]\ntype = \"local\"\n\n\
         [compile]\nfrontend = \"python\"\n"
    );
    write_new_file(&root.join("pack.toml"), &pack_toml)?;

    // agent.toml
    // NB: TOML has no way to return to top-level keys once a `[table]`
    // header has been opened, so every bare (non-table) key must come
    // before the first `[runtime]`/`[prompts]`/`[chat]` table header.
    let agent_toml = format!(
        "id = \"{id}\"\n\
         display_name = \"{display_name}\"\n\
         kind = \"conversational\"\n\
         domain = \"{id}\"\n\
         entry = \"{id}_agent.py\"\n\
         capabilities = []\n\
         skills = [\"{template_skill_id}\"]\n\n\
         [runtime]\n\
         loop = \"in_graph\"\n\
         memory_space = \"stm\"\n\
         session_prefix = \"{id}\"\n\n\
         [prompts]\n\
         persona = \"prompts/persona.md\"\n\n\
         [chat]\n\
         capability_discovery = true\n"
    );
    write_new_file(&root.join("agent.toml"), &agent_toml)?;

    // hierarchy.toml (optional per schema; scaffolded empty so authors see
    // the shape without being forced to declare a parent).
    write_new_file(
        &root.join("hierarchy.toml"),
        "# Optional: this agent's parent and the children it may spawn/delegate to.\n\
         # parent = \"team.supervisor\"\n\
         permitted_children = []\n",
    )?;

    // capabilities/
    write_new_file(
        &root.join("capabilities/capabilities.toml"),
        "# capability-definition.v1 entries. Every entry here must have a\n\
         # matching capabilities/permissions.toml entry (the joined capability)\n\
         # or `apxm package lint` fails.\n",
    )?;
    write_new_file(
        &root.join("capabilities/permissions.toml"),
        "# One [[permission]] per capabilities.toml entry (by id).\n",
    )?;

    // prompts/
    write_new_file(
        &root.join("prompts/persona.md"),
        &format!("# {display_name}\n\nDescribe this agent's persona here.\n"),
    )?;

    // python entry
    write_new_file(
        &root.join(format!("python/{id}_agent.py")),
        &format!(
            "\"\"\"{display_name} — entry point.\n\n\
             Entries are Python only (masterplan D2). Author this with either the\n\
             high-level ConversationalAgent wrapper or bare @compile/GraphRecorder.\n\
             \"\"\"\n\n\
             from apxm import compile, GraphRecorder\n\n\n\
             @compile()\n\
             def {id}_agent(g: GraphRecorder, message: str):\n\
             \x20\x20\x20\x20ask = g.ask(name=\"respond\", prompt=\"{{message}}\")\n\
             \x20\x20\x20\x20g.done(source=ask)\n"
        ),
    )?;

    // one example skill, prompt-only (compiled = false)
    write_new_file(
        &root.join(format!("skills/{template_skill_id}/skill.toml")),
        &format!(
            "id = \"{template_skill_id}\"\n\
             description = \"Template skill scaffolded by 'apxm package new'.\"\n\
             compiled = false\n\
             capabilities = []\n"
        ),
    )?;
    write_new_file(
        &root.join(format!("skills/{template_skill_id}/SKILL.md")),
        &format!("# {template_skill_id}\n\nDescribe what this skill does.\n"),
    )?;
    write_new_file(
        &root.join(format!("skills/{template_skill_id}/prompt.md")),
        "Prompt body for this skill goes here.\n",
    )?;

    // examples/ and tests/
    write_new_file(
        &root.join("examples/basic.md"),
        &format!("# Example\n\nA worked example for the `{id}` package.\n"),
    )?;
    write_new_file(
        &root.join("tests/README.md"),
        "Package tests (run by the linter/CI) go here.\n",
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
        print_section_header("Package Scaffolded");
        print_status_line(id, Status::Ok, &root.display().to_string());
        println!();
        println!("Next steps:");
        println!("  apxm package lint {}", root.display());
        println!("  apxm package build {}", root.display());
        println!("  apxm package install {}", root.display());
    }
    Ok(())
}

fn write_new_file(path: &Path, contents: &str) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("Failed to create {}", parent.display()))?;
    }
    fs::write(path, contents).with_context(|| format!("Failed to write {}", path.display()))
}

// ---------------------------------------------------------------------
// Shared manifest loading
// ---------------------------------------------------------------------

fn read_toml<T: serde::de::DeserializeOwned>(path: &Path) -> Result<T> {
    let text =
        fs::read_to_string(path).with_context(|| format!("Failed to read {}", path.display()))?;
    toml::from_str(&text).with_context(|| format!("Failed to parse {}", path.display()))
}

struct LoadedPackage {
    root: PathBuf,
    pack: PackToml,
    agent: AgentToml,
    hierarchy: Option<HierarchyToml>,
    capabilities: CapabilitiesToml,
    permissions: PermissionsToml,
    skills: Vec<SkillToml>,
}

fn load_package(root: &Path) -> Result<LoadedPackage> {
    if !root.is_dir() {
        bail!("'{}' is not a directory", root.display());
    }
    let pack_path = root.join("pack.toml");
    let agent_path = root.join("agent.toml");
    if !pack_path.is_file() {
        bail!("missing required file: {}", pack_path.display());
    }
    if !agent_path.is_file() {
        bail!("missing required file: {}", agent_path.display());
    }
    let pack: PackToml = read_toml(&pack_path)?;
    let agent: AgentToml = read_toml(&agent_path)?;

    let hierarchy_path = root.join("hierarchy.toml");
    let hierarchy = if hierarchy_path.is_file() {
        Some(read_toml(&hierarchy_path)?)
    } else {
        None
    };

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

    let mut skills = Vec::new();
    let skills_dir = root.join("skills");
    if skills_dir.is_dir() {
        let mut entries: Vec<_> = fs::read_dir(&skills_dir)
            .with_context(|| format!("Failed to read {}", skills_dir.display()))?
            .collect::<std::io::Result<Vec<_>>>()?;
        entries.sort_by_key(std::fs::DirEntry::path);
        for entry in entries {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            let skill_toml = path.join("skill.toml");
            if !skill_toml.is_file() {
                bail!(
                    "skill directory '{}' is missing required skill.toml",
                    path.display()
                );
            }
            skills.push(read_toml(&skill_toml)?);
        }
    }

    Ok(LoadedPackage {
        root: root.to_path_buf(),
        pack,
        agent,
        hierarchy,
        capabilities,
        permissions,
        skills,
    })
}

// ---------------------------------------------------------------------
// package lint
// ---------------------------------------------------------------------

/// Every path recognized by `agent-package.v1#/properties/files` (see the
/// JSON schema's `patternProperties`). Anything else fails validation ("no
/// package-private layout").
fn recognized_relpath(rel: &str) -> bool {
    if matches!(rel, "pack.toml" | "agent.toml" | "hierarchy.toml" | "README.md") {
        return true;
    }
    let parts: Vec<&str> = rel.split('/').collect();
    match parts.as_slice() {
        ["capabilities", "capabilities.toml" | "permissions.toml"] => true,
        ["capabilities", "handlers", f] => f.ends_with(".py"),
        ["prompts", f] => f.ends_with(".md"),
        ["python", f] => f.ends_with(".py"),
        ["skills", _id, "skill.toml"] => true,
        // SKILL.md / prompt.md are the primary prose; a skill may also carry
        // supplementary authored `.md` docs (e.g. a canvas/contract reference)
        // alongside them, plus its optional compiled artifacts.
        ["skills", _id, f] => {
            f.ends_with(".md") || *f == "skill.air" || *f == "skill.apxmobj"
        }
        ["skills", _id, "examples", f] => f.ends_with(".air"),
        ["examples", f] => f.ends_with(".md"),
        ["tests", f] => !f.is_empty(),
        ["shared", f] => !f.is_empty(),
        _ => false,
    }
}

/// Walk the package root and return every recognized file as a
/// (relative-path, absolute-path) pair, sorted by relative path.
fn walk_recognized_files(root: &Path) -> Result<Vec<(String, PathBuf)>> {
    let mut out = Vec::new();
    for entry in walkdir::WalkDir::new(root).into_iter() {
        let entry = entry.with_context(|| format!("Failed to walk {}", root.display()))?;
        if !entry.file_type().is_file() {
            continue;
        }
        let rel = entry
            .path()
            .strip_prefix(root)
            .expect("walkdir yields paths under root");
        let rel_str = rel.to_string_lossy().replace('\\', "/");
        if recognized_relpath(&rel_str) {
            out.push((rel_str, entry.path().to_path_buf()));
        }
    }
    out.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(out)
}

/// Find any file under the package root that is *not* recognized by the
/// schema's folder contract (excluding common noise: `.git`, `__pycache__`,
/// build sidecars).
fn find_unrecognized_files(root: &Path) -> Result<Vec<String>> {
    let mut out = Vec::new();
    for entry in walkdir::WalkDir::new(root).into_iter().filter_entry(|e| {
        let name = e.file_name().to_string_lossy();
        name != ".git" && name != "__pycache__" && name != ".pytest_cache"
    }) {
        let entry = entry.with_context(|| format!("Failed to walk {}", root.display()))?;
        if !entry.file_type().is_file() {
            continue;
        }
        let rel = entry
            .path()
            .strip_prefix(root)
            .expect("walkdir yields paths under root");
        let rel_str = rel.to_string_lossy().replace('\\', "/");
        if !recognized_relpath(&rel_str) {
            out.push(rel_str);
        }
    }
    out.sort();
    Ok(out)
}

/// The AGT-1 capability-set agreement check: agent.toml's declared
/// capabilities must resolve into `capabilities.toml`, every
/// `capabilities.toml` entry must have a matching `permissions.toml` entry
/// (the "joined capability" — otherwise it "is not a capability and fails
/// lint"), every `agent.toml` skill reference must be a real `skills/<id>/`
/// directory, and every skill's own declared capabilities must be in the
/// package's joined set. Returns human-readable error strings; empty = clean.
fn check_capability_drift(pkg: &LoadedPackage) -> Vec<String> {
    let mut errors = Vec::new();

    let declared_caps: BTreeSet<&str> = pkg
        .capabilities
        .capability
        .iter()
        .map(|c| c.id.as_str())
        .collect();
    let permitted_caps: BTreeSet<&str> = pkg
        .permissions
        .permission
        .iter()
        .map(|p| p.capability.as_str())
        .collect();

    // Joined capability: every capabilities.toml entry needs a permissions.toml entry.
    for cap in &declared_caps {
        if !permitted_caps.contains(cap) {
            errors.push(format!(
                "capability '{cap}' is declared in capabilities/capabilities.toml but has no \
                 matching entry in capabilities/permissions.toml — an entry with no permission \
                 policy is not a capability (AGT-1)"
            ));
        }
    }
    // A permission entry with no matching declaration is equally a drift: the
    // permission plane has no capability to attach to.
    for cap in &permitted_caps {
        if !declared_caps.contains(cap) {
            errors.push(format!(
                "capabilities/permissions.toml declares a policy for '{cap}', which is not \
                 defined in capabilities/capabilities.toml"
            ));
        }
    }

    let joined_caps: BTreeSet<&str> = declared_caps
        .intersection(&permitted_caps)
        .copied()
        .collect();

    // agent.toml capabilities must resolve into the joined set.
    for cap in &pkg.agent.capabilities {
        if !declared_caps.contains(cap.as_str()) {
            errors.push(format!(
                "agent.toml declares capability '{cap}' which is not defined in \
                 capabilities/capabilities.toml"
            ));
        } else if !permitted_caps.contains(cap.as_str()) {
            errors.push(format!(
                "agent.toml declares capability '{cap}' which has no permissions.toml entry"
            ));
        }
    }

    // agent.toml skills must be real skills/<id>/ directories.
    let skill_ids: BTreeSet<&str> = pkg.skills.iter().map(|s| s.id.as_str()).collect();
    for skill_ref in &pkg.agent.skills {
        if !skill_ids.contains(skill_ref.as_str()) {
            errors.push(format!(
                "agent.toml declares local skill '{skill_ref}' with no matching skills/{skill_ref}/ \
                 directory (or its skill.toml id does not match its directory name)"
            ));
        }
    }

    // Every skill's own capability references must be inside the joined set.
    for skill in &pkg.skills {
        for cap in &skill.capabilities {
            if !joined_caps.contains(cap.as_str()) {
                errors.push(format!(
                    "skill '{}' invokes capability '{cap}' which is not in the package's joined \
                     capability set (capabilities.toml + permissions.toml)",
                    skill.id
                ));
            }
        }
    }

    errors
}

/// Structural + required-field checks mirroring `agent-package.v1`'s
/// `required` arrays (`pack.pack_id`/`version`, `agent.id`/`entry`, the
/// `id == pack_id == agent.id` identity rule, `entry` must end in `.py`,
/// hooks must use a known event/mode).
fn check_schema_shape(pkg: &LoadedPackage) -> Vec<String> {
    let mut errors = Vec::new();

    if pkg.pack.pack_id.trim().is_empty() {
        errors.push("pack.toml: pack_id must not be empty".to_string());
    }
    if !semver_like(&pkg.pack.version) {
        errors.push(format!(
            "pack.toml: version '{}' is not valid SemVer",
            pkg.pack.version
        ));
    }
    if pkg.agent.id.trim().is_empty() {
        errors.push("agent.toml: id must not be empty".to_string());
    }
    if pkg.pack.pack_id != pkg.agent.id {
        errors.push(format!(
            "pack.toml pack_id ('{}') must equal agent.toml id ('{}')",
            pkg.pack.pack_id, pkg.agent.id
        ));
    }
    if !pkg.agent.entry.ends_with(".py") {
        errors.push(format!(
            "agent.toml: entry '{}' must be a .py path (entries are Python only)",
            pkg.agent.entry
        ));
    } else {
        let entry_path = pkg.root.join("python").join(&pkg.agent.entry);
        if !entry_path.is_file() {
            errors.push(format!(
                "agent.toml: entry '{}' does not exist at {}",
                pkg.agent.entry,
                entry_path.display()
            ));
        }
    }
    for hook in &pkg.agent.hooks {
        const VALID_EVENTS: &[&str] = &[
            "session_start",
            "pre_turn",
            "post_turn",
            "pre_ask",
            "post_ask",
            "pre_cap",
            "post_cap",
        ];
        if !VALID_EVENTS.contains(&hook.event.as_str()) {
            errors.push(format!("agent.toml: hook event '{}' is not a recognized lifecycle event", hook.event));
        }
        if hook.mode != "observe" && hook.mode != "gate" {
            errors.push(format!(
                "agent.toml: hook mode '{}' must be 'observe' or 'gate'",
                hook.mode
            ));
        }
    }
    if let Some(hierarchy) = &pkg.hierarchy {
        if let Some(parent) = &hierarchy.parent {
            if parent.trim().is_empty() {
                errors.push("hierarchy.toml: parent must not be empty when present".to_string());
            }
        }
    }
    for skill in &pkg.skills {
        if skill.id.trim().is_empty() {
            errors.push("a skill.toml has an empty id".to_string());
        }
        if skill.compiled {
            let skill_dir = pkg.root.join("skills").join(&skill.id);
            if !skill_dir.join("skill.air").is_file() && !skill_dir.join("skill.apxmobj").is_file() {
                errors.push(format!(
                    "skill '{}' declares compiled = true but has neither skill.air nor skill.apxmobj",
                    skill.id
                ));
            }
        }
    }

    errors
}

fn semver_like(version: &str) -> bool {
    // SemVer core: MAJOR.MINOR.PATCH, optionally with -prerelease/+build.
    let core = version.split(['-', '+']).next().unwrap_or(version);
    let parts: Vec<&str> = core.split('.').collect();
    parts.len() == 3 && parts.iter().all(|p| !p.is_empty() && p.chars().all(|c| c.is_ascii_digit()))
}

fn package_lint(path: &Path, json_output: bool) -> Result<()> {
    let pkg = load_package(path)?;

    let mut errors = check_schema_shape(&pkg);
    errors.extend(check_capability_drift(&pkg));
    for unrecognized in find_unrecognized_files(path)? {
        errors.push(format!(
            "unrecognized file '{unrecognized}' is not part of the agent-package.v1 folder contract"
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
        print_section_header("Package Lint");
        if errors.is_empty() {
            print_status_line(&pkg.agent.id, Status::Ok, "no drift, schema-valid");
        } else {
            for error in &errors {
                print_status_line(&pkg.agent.id, Status::Error, error);
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
// package build — integrity hash chain (AGT-1 `integrity` schema)
// ---------------------------------------------------------------------

fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    let digest = hasher.finalize();
    digest.iter().map(|b| format!("{b:02x}")).collect()
}

/// Genesis `prev_hash` for the first chain link — 64 zeros, per AGT-1.
const GENESIS_HASH: &str = "0000000000000000000000000000000000000000000000000000000000000000";
const _GENESIS_HASH_IS_64_HEX_CHARS: () = assert!(GENESIS_HASH.len() == 64);

/// Compute the AGT-1 integrity hash chain over `files` (relpath -> content
/// digest), in ascending relpath order. Each link's hash is
/// `sha256(prev_hash || path || file_digest)` (UTF-8 byte concatenation of
/// the two hex digests and the path string); `chain[0].prev_hash` is 64
/// zeros (genesis); `pack_hash` is the final link's hash.
fn compute_integrity(files: &BTreeMap<String, String>) -> IntegrityToml {
    let genesis = GENESIS_HASH;
    let mut prev = genesis.to_string();
    let mut chain = Vec::with_capacity(files.len());
    for (path, digest) in files {
        let mut preimage = String::with_capacity(prev.len() + path.len() + digest.len());
        preimage.push_str(&prev);
        preimage.push_str(path);
        preimage.push_str(digest);
        let hash = sha256_hex(preimage.as_bytes());
        chain.push(ChainLinkToml {
            path: path.clone(),
            prev_hash: prev.clone(),
            hash: hash.clone(),
        });
        prev = hash;
    }
    let pack_hash = chain
        .last()
        .map(|l| l.hash.clone())
        .unwrap_or_else(|| genesis.to_string());
    IntegrityToml {
        algorithm: "sha256".to_string(),
        pack_hash,
        chain,
    }
}

/// Digest of `pack.toml` for the integrity chain, with the `[integrity]`
/// table itself stripped first — `pack_hash` cannot cover its own value.
/// Mirrors `compile.rs::strip_artifact_hash_for_embed`'s "strip before hash"
/// pattern for the same reason (`skill.toml`'s `artifact_hash` field there).
/// Because this strip is applied on every (re)build, the digest is stable
/// across rebuilds regardless of whatever stale `[integrity]` table happens
/// to already be on disk, which is what makes the chain independently
/// reproducible/verifiable from the finished package.
fn pack_toml_digest_for_chain(pack: &PackToml) -> Result<String> {
    let mut stripped = pack.clone();
    stripped.integrity = None;
    let text = toml::to_string_pretty(&stripped).context("Failed to serialize pack.toml")?;
    Ok(sha256_hex(text.as_bytes()))
}

/// Recompute a digest map from a package root's currently-recognized files
/// (used both to build and to independently verify a hash chain). `pack.toml`
/// is special-cased through [`pack_toml_digest_for_chain`].
fn digest_recognized_files(root: &Path) -> Result<BTreeMap<String, String>> {
    let mut files = BTreeMap::new();
    for (rel, abs) in walk_recognized_files(root)? {
        if rel == "pack.toml" {
            let pack: PackToml = read_toml(&abs)?;
            files.insert(rel, pack_toml_digest_for_chain(&pack)?);
            continue;
        }
        let bytes =
            fs::read(&abs).with_context(|| format!("Failed to read {}", abs.display()))?;
        files.insert(rel, sha256_hex(&bytes));
    }
    Ok(files)
}

// Compiling each skill's Python-frontend source into `skill.air`/
// `skill.apxmobj` would reuse the same pipeline `apxm compile` already
// drives (`apxm_driver::compiler::Compiler` + the AIR emission shellout in
// `commands::compile::emit_air_from_python`), but that pipeline is entirely
// behind the `driver` feature and its entry points are not factored for
// reuse against an arbitrary source path today (they're wired to
// `compile_command`'s CLI-argument shape). Wiring an actual compile-skill
// step is DEFERRED here (see the final report) rather than risked as a
// partially-tested reimplementation; `package build` always performs the
// part that is fully implemented and tested regardless of build features:
// computing and writing the AGT-1 integrity hash chain over the package's
// current on-disk files. Skills stay exactly as compiled/prompt-only as
// they were declared in their own `skill.toml`.
fn package_build(path: &Path, json_output: bool) -> Result<()> {
    let mut pkg = load_package(path)?;
    let compiled_skills: Vec<String> = Vec::new();

    let files = digest_recognized_files(path)?;
    let integrity = compute_integrity(&files);

    pkg.pack.integrity = Some(integrity.clone());
    let pack_toml_text =
        toml::to_string_pretty(&pkg.pack).context("Failed to serialize pack.toml")?;
    fs::write(path.join("pack.toml"), pack_toml_text)
        .with_context(|| format!("Failed to write {}", path.join("pack.toml").display()))?;

    if json_output {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "path": path.display().to_string(),
                "files": files.len(),
                "pack_hash": integrity.pack_hash,
                "compiled_skills": compiled_skills,
            }))?
        );
    } else {
        print_section_header("Package Build");
        print_status_line(
            &pkg.agent.id,
            Status::Ok,
            &format!(
                "{} files hashed, pack_hash={}",
                files.len(),
                integrity.pack_hash
            ),
        );
        if !compiled_skills.is_empty() {
            println!("  compiled skills: {}", compiled_skills.join(", "));
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------
// package install
// ---------------------------------------------------------------------

pub(crate) fn packages_dir(apxm_home: &Path) -> PathBuf {
    apxm_home.join("packages")
}

/// Resolve an installed agent-package's `pack.toml` under
/// `APXM_HOME/packages/<id>/` — the same install destination
/// [`package_install_to`] writes to. Shared with `org lint`'s member
/// resolution check (ORG-2) so org-package member references are checked
/// against the same install layout AGT-2's `package install` created,
/// rather than a second hand-rolled resolution path.
pub(crate) fn resolve_installed_package(apxm_home: &Path, id: &str) -> Result<PackToml> {
    let pack_path = packages_dir(apxm_home).join(id).join("pack.toml");
    if !pack_path.is_file() {
        bail!(
            "agent package '{id}' is not installed under {} (run 'apxm package install' first)",
            packages_dir(apxm_home).display()
        );
    }
    read_toml(&pack_path)
}

/// Recursively copy a directory tree. Shared by `package install` and
/// `org install` (both copy a validated source folder verbatim to an
/// `APXM_HOME` subtree).
pub(crate) fn copy_dir_recursive(src: &Path, dst: &Path) -> Result<()> {
    fs::create_dir_all(dst).with_context(|| format!("Failed to create {}", dst.display()))?;
    for entry in fs::read_dir(src).with_context(|| format!("Failed to read {}", src.display()))? {
        let entry = entry?;
        let file_type = entry.file_type()?;
        let src_path = entry.path();
        let dst_path = dst.join(entry.file_name());
        if file_type.is_dir() {
            copy_dir_recursive(&src_path, &dst_path)?;
        } else if file_type.is_file() {
            fs::copy(&src_path, &dst_path).with_context(|| {
                format!("Failed to copy {} to {}", src_path.display(), dst_path.display())
            })?;
        }
    }
    Ok(())
}

fn package_install(path: &Path, force: bool, json_output: bool) -> Result<()> {
    package_install_to(path, &apxm_core::env::apxm_home(), force, json_output)
}

/// Installs a package under an explicit `apxm_home` root. Split out from
/// [`package_install`] so tests can point at a tempdir instead of mutating
/// the process-global `APXM_HOME` env var (this crate denies `unsafe_code`,
/// which `std::env::set_var` requires in Rust 2024).
pub(crate) fn package_install_to(
    path: &Path,
    apxm_home: &Path,
    force: bool,
    json_output: bool,
) -> Result<()> {
    let pkg = load_package(path)?;
    let dest = packages_dir(apxm_home).join(&pkg.agent.id);

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
        .with_context(|| format!("Failed to create {}", packages_dir(apxm_home).display()))?;
    copy_dir_recursive(path, &dest)?;

    if json_output {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "id": pkg.agent.id,
                "installed_to": dest.display().to_string(),
            }))?
        );
    } else {
        print_section_header("Package Installed");
        print_status_line(&pkg.agent.id, Status::Ok, &dest.display().to_string());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn scaffold(dir: &Path, id: &str) {
        package_new(id, Some(dir.to_path_buf()), None, true).expect("scaffold ok");
    }

    #[test]
    fn new_scaffolds_schema_valid_tree() {
        let tmp = tempdir().unwrap();
        let root = tmp.path().join("demo");
        scaffold(&root, "demo");

        for rel in [
            "pack.toml",
            "agent.toml",
            "hierarchy.toml",
            "capabilities/capabilities.toml",
            "capabilities/permissions.toml",
            "prompts/persona.md",
            "python/demo_agent.py",
            "skills/demo-skill/skill.toml",
            "skills/demo-skill/SKILL.md",
            "skills/demo-skill/prompt.md",
            "examples/basic.md",
            "tests/README.md",
        ] {
            assert!(root.join(rel).is_file(), "missing scaffolded file: {rel}");
        }

        let pack: PackToml = read_toml(&root.join("pack.toml")).unwrap();
        assert_eq!(pack.pack_id, "demo");
        assert_eq!(pack.version, "0.1.0");

        let agent: AgentToml = read_toml(&root.join("agent.toml")).unwrap();
        assert_eq!(agent.id, "demo");
        assert_eq!(agent.entry, "demo_agent.py");
        assert_eq!(agent.skills, vec!["demo-skill".to_string()]);

        let skill: SkillToml = read_toml(&root.join("skills/demo-skill/skill.toml")).unwrap();
        assert_eq!(skill.id, "demo-skill");
        assert!(!skill.compiled);

        // The freshly scaffolded tree must lint clean (no capability
        // declared => nothing to join, no drift).
        package_lint(&root, true).expect("scaffolded package should lint clean");
    }

    #[test]
    fn new_refuses_nonempty_destination() {
        let tmp = tempdir().unwrap();
        let root = tmp.path().join("demo");
        scaffold(&root, "demo");
        let err = package_new("demo", Some(root.clone()), None, true).unwrap_err();
        assert!(err.to_string().contains("already exists"));
    }

    #[test]
    fn lint_passes_on_consistent_package() {
        let tmp = tempdir().unwrap();
        let root = tmp.path().join("clean");
        scaffold(&root, "clean");

        // Add one joined capability referenced by agent.toml and used by the skill.
        fs::write(
            root.join("capabilities/capabilities.toml"),
            "[[capability]]\nid = \"clean.example\"\ndescription = \"demo\"\n",
        )
        .unwrap();
        fs::write(
            root.join("capabilities/permissions.toml"),
            "[[permission]]\ncapability = \"clean.example\"\ndecision = \"allow\"\n",
        )
        .unwrap();
        let agent_toml = fs::read_to_string(root.join("agent.toml")).unwrap();
        fs::write(
            root.join("agent.toml"),
            agent_toml.replace("capabilities = []", "capabilities = [\"clean.example\"]"),
        )
        .unwrap();
        fs::write(
            root.join("skills/clean-skill/skill.toml"),
            "id = \"clean-skill\"\ncompiled = false\ncapabilities = [\"clean.example\"]\n",
        )
        .unwrap();

        package_lint(&root, true).expect("consistent package should lint clean");
    }

    #[test]
    fn lint_catches_capability_set_drift() {
        let tmp = tempdir().unwrap();
        let root = tmp.path().join("drift");
        scaffold(&root, "drift");

        // agent.toml references a capability that capabilities.toml never declares.
        let agent_toml = fs::read_to_string(root.join("agent.toml")).unwrap();
        fs::write(
            root.join("agent.toml"),
            agent_toml.replace("capabilities = []", "capabilities = [\"drift.undeclared\"]"),
        )
        .unwrap();

        let err = package_lint(&root, true).expect_err("drift must fail lint");
        assert!(err.to_string().contains("lint error"));

        // Now declare the capability but withhold its permissions entry —
        // "not a capability" per AGT-1.
        fs::write(
            root.join("capabilities/capabilities.toml"),
            "[[capability]]\nid = \"drift.undeclared\"\n",
        )
        .unwrap();
        let err = package_lint(&root, true).expect_err("missing permission entry must fail lint");
        assert!(
            err.to_string().contains("lint error"),
            "expected a lint error, got: {err}"
        );
    }

    #[test]
    fn lint_catches_skill_capability_drift() {
        let tmp = tempdir().unwrap();
        let root = tmp.path().join("skill-drift");
        scaffold(&root, "skill-drift");
        fs::write(
            root.join("skills/skill-drift-skill/skill.toml"),
            "id = \"skill-drift-skill\"\ncompiled = false\ncapabilities = [\"nowhere.declared\"]\n",
        )
        .unwrap();
        let err = package_lint(&root, true).expect_err("undeclared skill capability must fail lint");
        assert!(err.to_string().contains("lint error"));
    }

    #[test]
    fn lint_rejects_unrecognized_files() {
        let tmp = tempdir().unwrap();
        let root = tmp.path().join("stray");
        scaffold(&root, "stray");
        fs::write(root.join("not-a-real-file.txt"), "nope").unwrap();
        let err = package_lint(&root, true).expect_err("unrecognized file must fail lint");
        assert!(err.to_string().contains("lint error"));
    }

    #[test]
    fn build_produces_verifiable_hash_chain() {
        let tmp = tempdir().unwrap();
        let root = tmp.path().join("hashed");
        scaffold(&root, "hashed");

        package_build(&root, true).expect("build ok");

        let pack: PackToml = read_toml(&root.join("pack.toml")).unwrap();
        let integrity = pack.integrity.expect("build must write [integrity]");
        assert_eq!(integrity.algorithm, "sha256");
        assert!(!integrity.chain.is_empty());
        assert_eq!(
            integrity.chain.last().unwrap().hash,
            integrity.pack_hash,
            "pack_hash must equal the final chain link's hash"
        );

        // Independently recompute every file's digest straight from the
        // finished package on disk (pack.toml's own digest is derived with
        // its `[integrity]` table stripped first, same as build did — see
        // `pack_toml_digest_for_chain`) and confirm the whole chain
        // algebra: each link's hash == sha256(prev_hash || path || digest),
        // link 0's prev_hash is genesis, and pack_hash is the last link.
        let files = digest_recognized_files(&root).expect("recompute digests");
        assert_eq!(files.len(), integrity.chain.len());

        for (i, link) in integrity.chain.iter().enumerate() {
            let digest = files
                .get(&link.path)
                .unwrap_or_else(|| panic!("missing digest for {}", link.path));
            let mut preimage = String::new();
            preimage.push_str(&link.prev_hash);
            preimage.push_str(&link.path);
            preimage.push_str(digest);
            let expected_hash = sha256_hex(preimage.as_bytes());
            assert_eq!(
                expected_hash, link.hash,
                "chain link {i} ({}) hash mismatch on independent recomputation",
                link.path
            );
            if i == 0 {
                assert_eq!(link.prev_hash, GENESIS_HASH);
            } else {
                assert_eq!(link.prev_hash, integrity.chain[i - 1].hash);
            }
        }

        // A tampered file must break the chain: mutate one hashed file and
        // confirm the recomputed digest at that path no longer matches the
        // hash recorded in the (untouched) chain.
        fs::write(root.join("prompts/persona.md"), "tampered content\n").unwrap();
        let tampered_files = digest_recognized_files(&root).expect("recompute after tamper");
        let persona_link = integrity
            .chain
            .iter()
            .find(|l| l.path == "prompts/persona.md")
            .expect("persona.md is in the chain");
        let mut preimage = String::new();
        preimage.push_str(&persona_link.prev_hash);
        preimage.push_str(&persona_link.path);
        preimage.push_str(&tampered_files["prompts/persona.md"]);
        assert_ne!(
            sha256_hex(preimage.as_bytes()),
            persona_link.hash,
            "tampering with a hashed file must invalidate its chain link"
        );
    }

    #[test]
    fn install_places_files_under_apxm_home() {
        let tmp = tempdir().unwrap();
        let root = tmp.path().join("installable");
        scaffold(&root, "installable");
        let fake_home = tempdir().unwrap();

        // Point installs at a tempdir explicitly (never mutates the process
        // env / real home — see `package_install_to`'s doc comment).
        package_install_to(&root, fake_home.path(), false, true).expect("install ok");

        let dest = fake_home.path().join("packages").join("installable");
        assert!(dest.join("pack.toml").is_file());
        assert!(dest.join("agent.toml").is_file());
        assert!(dest.join("skills/installable-skill/skill.toml").is_file());

        // The real home directory must never be touched by this test.
        let real_home = dirs::home_dir().unwrap_or_default();
        assert!(!real_home.join(".apxm/packages/installable").exists());
    }

    #[test]
    fn install_refuses_existing_destination_without_force() {
        let tmp = tempdir().unwrap();
        let root = tmp.path().join("dupe");
        scaffold(&root, "dupe");
        let fake_home = tempdir().unwrap();

        package_install_to(&root, fake_home.path(), false, true).expect("first install ok");
        let second = package_install_to(&root, fake_home.path(), false, true);
        let err = second.expect_err("second install without --force must fail");
        assert!(err.to_string().contains("already exists"));
    }

    #[test]
    fn install_ignores_manual_libs_dir() {
        // AGT-4 hard cutover: `~/.apxm/libs` is no longer a load or scan root.
        // Install must place the pack under packages/ and neither read nor touch
        // any stale copy sitting under the old libs convention.
        let tmp = tempdir().unwrap();
        let root = tmp.path().join("conflicted");
        scaffold(&root, "conflicted");
        let fake_home = tempdir().unwrap();

        let manual = fake_home.path().join("libs").join("conflicted");
        fs::create_dir_all(&manual).unwrap();
        fs::write(manual.join("marker.txt"), "stale libs copy, left untouched").unwrap();

        package_install_to(&root, fake_home.path(), false, true).expect("install ok");

        assert!(manual.join("marker.txt").is_file());
        assert!(
            fake_home
                .path()
                .join("packages/conflicted/pack.toml")
                .is_file()
        );
    }
}
