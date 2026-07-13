//! `apxm agent new|sync|lint|build|install` — the toolchain for the
//! canonical agent folder format (`apxm.agent.v1`).
//!
//! The folder contract, required/optional files, and integrity hash-chain
//! algorithm are hand-ported from that schema into the Rust checks below
//! rather than loading the JSON schema file at runtime.
//!
//! Capability-set drift (agent.toml vs capabilities/capabilities.toml vs
//! capabilities/permissions.toml vs skills/) is enforced as a lint ERROR
//! as a agent invariant. No hand-rolled equivalent of this check
//! was found elsewhere in the workspace at the schema's grammar (the
//! only existing "undeclared capability" check is
//! `server/crates/core/src/skills.rs::validate_inv_cap_node`, which compares
//! a *compiled skill artifact's* INV_CAP nodes against a manifest's
//! `declared_tools` set — a different, narrower check over compiled AIR, not
//! over the agent's authored `capabilities.toml`/`permissions.toml`/
//! `agent.toml` triple). This module's drift check is therefore a fresh
//! implementation of the  "joined capability" semantics described in
//! the agent schema, not a port of that server check.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow, bail};
use apxm_core::constants::env as apxm_env;
use apxm_core::types::{HandlerKind, HandlerLanguage, HandlerManifest};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

#[cfg(feature = "driver")]
use apxm_driver::compiler::Compiler;

use super::implementations::{Status, print_section_header, print_status_line};

// ---------------------------------------------------------------------
// On-disk manifest shapes (apxm.agent.v1 projections)
// ---------------------------------------------------------------------

const AGENT_SCHEMA_V1: &str = "apxm.agent.v1";

/// Projection of generated-only `integrity.toml`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IntegrityToml {
    pub algorithm: String,
    pub hash: String,
    pub chain: Vec<ChainLinkToml>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChainLinkToml {
    pub path: String,
    pub prev_hash: String,
    pub hash: String,
}

/// Projection of `agent.toml` (`apxm.agent.v1#/properties/agent`).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentToml {
    pub id: String,
    pub version: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub schema_version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub license: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<toml::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub compile: Option<CompileToml>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub domain: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runtime: Option<RuntimeToml>,
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

/// `[runtime.loop]` table for recv/re-arm looped agents.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuntimeLoopTable {
    pub mode: String,
    #[serde(default)]
    pub rearm: bool,
    pub turn_param: String,
}

/// `[runtime]` loop: string (`host`/`in_graph`) or nested `[runtime.loop]`
/// table (`mode`, `rearm`, `turn_param`).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum LoopToml {
    StringMode(String),
    Config(RuntimeLoopTable),
}

/// `agent.toml`'s `[runtime]` table — the single source of truth for every
/// `ConversationalAgent` knob. `loop`/`memory_space`/`session_prefix`
/// are the knobs known today; `extra` keeps the table forward-compatible
/// with knobs added later without a schema break (same open-shape rule as `CapabilityEntry`/
/// `PermissionEntry`'s `#[serde(flatten)] extra` pattern above).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RuntimeToml {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#loop: Option<LoopToml>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub memory_space: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_prefix: Option<String>,
    #[serde(flatten)]
    pub extra: toml::Table,
}

fn runtime_has_loop(runtime: &RuntimeToml) -> bool {
    match &runtime.r#loop {
        Some(LoopToml::StringMode(value)) => !value.trim().is_empty(),
        Some(LoopToml::Config(_)) => true,
        None => false,
    }
}

fn validate_runtime_loop(runtime: &RuntimeToml) -> Option<String> {
    match &runtime.r#loop {
        Some(LoopToml::StringMode(loop_mode)) => {
            if loop_mode != "host" && loop_mode != "in_graph" {
                Some(format!(
                    "agent.toml: [runtime] loop '{loop_mode}' must be 'host' or 'in_graph'"
                ))
            } else {
                None
            }
        }
        Some(LoopToml::Config(table)) => {
            if table.mode.trim().is_empty() {
                Some("agent.toml: [runtime.loop] mode must not be empty".to_string())
            } else if table.turn_param.trim().is_empty() {
                Some("agent.toml: [runtime.loop] turn_param must not be empty".to_string())
            } else {
                None
            }
        }
        None => None,
    }
}

/// Projection of the optional `hierarchy.toml`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct HierarchyToml {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub permitted_children: Vec<String>,
}

/// `capabilities/capabilities.toml` — array of agent capability ids used by
/// agent lint.
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
/// id (the  "joined capability" rule: every `capabilities.toml` entry
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

/// Source language accepted by the supported compiler frontends.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum FrontendLanguage {
    /// Python frontend source.
    Python,
    /// TypeScript frontend source.
    TypeScript,
}

impl FrontendLanguage {
    /// File extension required for an authored package entry.
    pub const fn source_extension(self) -> &'static str {
        match self {
            Self::Python => ".py",
            Self::TypeScript => ".ts",
        }
    }

    /// Required source filename for a compiled skill.
    pub const fn skill_source_filename(self) -> &'static str {
        match self {
            Self::Python => "skill.py",
            Self::TypeScript => "skill.ts",
        }
    }
}

impl fmt::Display for FrontendLanguage {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Python => "python",
            Self::TypeScript => "typescript",
        })
    }
}

/// Resolve an entry from the installed TypeScript frontend package.
pub(super) fn installed_typescript_frontend_entry(relative: &str) -> Result<PathBuf> {
    let package =
        std::env::var_os(apxm_env::APXM_TYPESCRIPT_FRONTEND_PACKAGE).ok_or_else(|| {
            anyhow!(
                "{} must point to the installed @apxm/frontend package",
                apxm_env::APXM_TYPESCRIPT_FRONTEND_PACKAGE,
            )
        })?;
    let entry = PathBuf::from(package).join(relative);
    if !entry.is_file() {
        bail!(
            "installed @apxm/frontend entry is missing: {}",
            entry.display()
        );
    }
    Ok(entry)
}

/// Source-bearing package declaration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CompileToml {
    /// Package-relative frontend entry path.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub entry: Option<String>,
    /// Frontend that owns the entry source syntax.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub frontend: Option<FrontendLanguage>,
}

/// `skills/<id>/skill.toml` — always present per skill.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillToml {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// `false` (default) => prompt-only (`CompileStatus::NotCompiled`).
    #[serde(default)]
    pub compiled: bool,
    /// Capability ids this skill invokes; each must be in the agent's
    /// joined capability set.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub capabilities: Vec<String>,
    /// Frontend that authored the compiled source. Required when `compiled`
    /// is true and absent for prompt-only skills.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub frontend: Option<FrontendLanguage>,
    /// sha256 hex of the frontend source file's bytes as of the last
    /// successful `agent build` compile of this skill. `None` before the
    /// first successful compiled build (or for prompt-only skills).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_hash: Option<String>,
    /// sha256 hex of `skill.air`'s bytes as written by that same build.
    /// Paired with `source_hash` to detect a hand-edited artifact: see
    /// [`detect_hand_edited_artifact`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub air_hash: Option<String>,
}

// ---------------------------------------------------------------------
// agent new
// ---------------------------------------------------------------------

pub fn agent_command(action: super::AgentAction, json_output: bool) -> Result<()> {
    match action {
        super::AgentAction::New {
            id,
            path,
            display_name,
            template,
        } => agent_new(&id, path, display_name, &template, json_output),
        super::AgentAction::Sync { path } => agent_sync(&path, json_output),
        super::AgentAction::Lint { path, org } => agent_lint(&path, org, json_output),
        super::AgentAction::Build { path } => agent_build(&path, json_output),
        super::AgentAction::Install { path, force } => agent_install(&path, force, json_output),
    }
}

fn default_agent_root(id: &str) -> PathBuf {
    PathBuf::from("agents").join(id)
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

pub(crate) fn agent_new(
    id: &str,
    path: Option<PathBuf>,
    display_name: Option<String>,
    template: &str,
    json_output: bool,
) -> Result<()> {
    if id.trim().is_empty() {
        bail!("agent id must not be empty");
    }
    let root = path.unwrap_or_else(|| default_agent_root(id));
    if root.exists() && fs::read_dir(&root)?.next().is_some() {
        bail!(
            "destination '{}' already exists and is not empty",
            root.display()
        );
    }

    if template == "looped-agent" {
        return agent_new_looped_agent(id, &root, display_name, json_output);
    }

    let template_dir = resolve_agent_template_dir(template)?;
    agent_new_from_template_dir(id, &root, display_name, &template_dir, json_output)
}

fn agent_examples_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../examples/agents")
}

pub(crate) fn example_agent_dir(name: &str) -> PathBuf {
    agent_examples_dir().join(name)
}

#[cfg(test)]
pub(crate) fn gao_example_agent_dir() -> PathBuf {
    example_agent_dir("gao")
}

fn resolve_agent_template_dir(template: &str) -> Result<PathBuf> {
    let requested = PathBuf::from(template);
    if requested.is_dir() {
        return Ok(requested);
    }

    if requested.components().count() == 1 {
        let example = example_agent_dir(template);
        if example.is_dir() {
            return Ok(example);
        }
    }

    bail!(
        "unknown agent template '{template}': expected 'looped-agent', an example name under {}, or a directory path",
        agent_examples_dir().display()
    )
}

fn agent_new_from_template_dir(
    id: &str,
    root: &Path,
    display_name: Option<String>,
    template_dir: &Path,
    json_output: bool,
) -> Result<()> {
    if !template_dir.join("agent.toml").is_file() {
        bail!(
            "agent template '{}' is missing required agent.toml",
            template_dir.display()
        );
    }
    copy_dir_recursive(template_dir, root)?;
    let copied_integrity = root.join("integrity.toml");
    if copied_integrity.is_file() {
        fs::remove_file(&copied_integrity)
            .with_context(|| format!("Failed to remove {}", copied_integrity.display()))?;
    }
    rewrite_scaffolded_identity(root, id, display_name)?;
    agent_sync(root, json_output)?;
    print_agent_scaffolded(id, root, json_output)
}

fn rewrite_scaffolded_identity(root: &Path, id: &str, display_name: Option<String>) -> Result<()> {
    let display_name = display_name.unwrap_or_else(|| titleize(id));
    let agent_path = root.join("agent.toml");
    let mut agent: AgentToml = read_toml(&agent_path)?;
    agent.id = id.to_string();
    agent.display_name = Some(display_name);
    if let Some(runtime) = agent.runtime.as_mut() {
        runtime.session_prefix = Some(id.to_string());
    }
    fs::write(&agent_path, toml::to_string_pretty(&agent)?)?;
    Ok(())
}

fn agent_new_looped_agent(
    id: &str,
    root: &Path,
    display_name: Option<String>,
    json_output: bool,
) -> Result<()> {
    let display_name = display_name.unwrap_or_else(|| titleize(id));
    let template_skill_id = format!("{id}-skill");

    write_new_file(
        &root.join("agent.toml"),
        &format!(
            "id = \"{id}\"\n\
             version = \"0.1.0\"\n\
             schema_version = \"{AGENT_SCHEMA_V1}\"\n\
             license = \"MIT\"\n\
             display_name = \"{display_name}\"\n\
             kind = \"agent\"\n\
             domain = \"{id}\"\n\
             capabilities = []\n\
             skills = [\"{template_skill_id}\"]\n\n\
             [runtime]\n\
             memory_space = \"stm\"\n\
             session_prefix = \"{id}\"\n\n\
             [runtime.loop]\n\
             mode = \"recv\"\n\
             rearm = true\n\
             turn_param = \"user_message\"\n\n\
             [prompts]\n\
             persona = \"prompts/persona.md\"\n\n\
             [chat]\n\
             capability_discovery = true\n\n\
             [source]\n\
             type = \"local\"\n"
        ),
    )?;

    write_new_file(
        &root.join("hierarchy.toml"),
        "# Optional: this agent's parent and the children it may spawn/delegate to.\n\
         # parent = \"team.supervisor\"\n\
         permitted_children = []\n",
    )?;

    write_new_file(
        &root.join("package.json"),
        &format!("{{\n  \"name\": \"{id}\",\n  \"private\": true,\n  \"type\": \"module\"\n}}\n"),
    )?;
    write_new_file(
        &root.join("tsconfig.json"),
        "{\n  \"compilerOptions\": {\n    \"target\": \"ES2022\",\n    \"module\": \"ESNext\",\n    \"moduleResolution\": \"bundler\",\n    \"strict\": true\n  }\n}\n",
    )?;

    scaffold_capability_folder(
        root,
        "read",
        "Read a file under declared read roots.",
        "builtin",
        None,
        true,
        "allow",
    )?;
    scaffold_capability_folder(
        root,
        "write",
        "Write a file under declared write roots.",
        "builtin",
        None,
        false,
        "ask",
    )?;

    write_new_file(
        &root.join("capabilities/handlers/hooks.ts"),
        "// Sample hook handlers for a looped agent.\n\
         export function inject_context(_ctx: unknown): null {\n  return null;\n}\n",
    )?;

    write_new_file(
        &root.join("prompts/persona.md"),
        &format!("# {display_name}\n\nDescribe this agent's persona here.\n"),
    )?;

    write_new_file(
        &root.join(format!("skills/{template_skill_id}/skill.toml")),
        &format!(
            "id = \"{template_skill_id}\"\n\
             description = \"Template skill scaffolded by 'apxm agent new'.\"\n\
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

    write_new_file(
        &root.join("examples/basic.md"),
        &format!("# Example\n\nA worked example for the `{id}` agent.\n"),
    )?;
    write_new_file(
        &root.join("tests/README.md"),
        "Agent tests (run by the linter/CI) go here.\n",
    )?;

    agent_sync(root, json_output)?;
    print_agent_scaffolded(id, root, json_output)
}

fn scaffold_capability_folder(
    root: &Path,
    cap_id: &str,
    description: &str,
    kind: &str,
    builtin_group: Option<&str>,
    read_only: bool,
    decision: &str,
) -> Result<()> {
    let dir = root.join("capabilities").join(cap_id);
    let builtin_group_line = builtin_group
        .map(|group| format!("builtin_group = \"{group}\"\n"))
        .unwrap_or_default();
    write_new_file(
        &dir.join("capability.toml"),
        &format!(
            "id = \"{cap_id}\"\n\
             description = \"{description}\"\n\
             kind = \"{kind}\"\n\
             {builtin_group_line}\
             read_only = {read_only}\n"
        ),
    )?;
    write_new_file(
        &dir.join("permission.toml"),
        &format!("capability = \"{cap_id}\"\ndecision = \"{decision}\"\n"),
    )?;
    Ok(())
}

fn print_agent_scaffolded(id: &str, root: &Path, json_output: bool) -> Result<()> {
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
        print_section_header("Agent Scaffolded");
        print_status_line(id, Status::Ok, &root.display().to_string());
        println!();
        println!("Next steps:");
        println!("  apxm agent sync {}", root.display());
        println!("  apxm agent lint {}", root.display());
        println!("  apxm agent build {}", root.display());
        println!("  apxm agent install {}", root.display());
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

const SYNC_GENERATED_HEADER: &str = "# Generated by apxm agent sync; edit folders instead.\n";

fn capability_kind(entry: &CapabilityEntry) -> Option<&str> {
    entry.extra.get("kind").and_then(|value| value.as_str())
}

fn validate_flat_capability_id(id: &str, cap_id: &str, folder: &str) -> Result<()> {
    if cap_id != folder {
        bail!(
            "capability folder '{folder}' declares id '{cap_id}' in capability.toml; ids must match their folder name"
        );
    }
    if cap_id.contains('.') {
        bail!(
            "capability id '{cap_id}' must be flat (no '.' segments); edit capabilities/{folder}/ instead"
        );
    }
    let prefix = format!("{id}.");
    if cap_id.starts_with(&prefix) {
        bail!("capability id '{cap_id}' must not embed the agent id prefix '{id}.'; use flat ids");
    }
    Ok(())
}

fn scan_capability_folders(
    root: &Path,
    id: &str,
) -> Result<(Vec<CapabilityEntry>, Vec<PermissionEntry>)> {
    let caps_dir = root.join("capabilities");
    if !caps_dir.is_dir() {
        return Ok((Vec::new(), Vec::new()));
    }

    let mut capabilities = Vec::new();
    let mut permissions = Vec::new();
    let mut entries: Vec<_> = fs::read_dir(&caps_dir)
        .with_context(|| format!("Failed to read {}", caps_dir.display()))?
        .collect::<std::io::Result<Vec<_>>>()?;
    entries.sort_by_key(|entry| entry.path());

    for entry in entries {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let folder = entry.file_name().to_string_lossy().into_owned();
        if folder == "handlers" {
            continue;
        }

        let cap_path = path.join("capability.toml");
        let perm_path = path.join("permission.toml");
        if !cap_path.is_file() {
            bail!(
                "capability folder '{}' is missing required capability.toml",
                path.display()
            );
        }
        if !perm_path.is_file() {
            bail!(
                "capability folder '{}' is missing required permission.toml",
                path.display()
            );
        }

        let cap: CapabilityEntry = read_toml(&cap_path)?;
        let perm: PermissionEntry = read_toml(&perm_path)?;
        validate_flat_capability_id(id, &cap.id, &folder)?;
        if perm.capability != cap.id {
            bail!(
                "capability folder '{folder}': permission.toml capability '{}' does not match capability id '{}'",
                perm.capability,
                cap.id
            );
        }
        if capability_kind(&cap) == Some("typescript_handler") && !path.join("handler.ts").is_file()
        {
            bail!(
                "capability '{folder}' has kind = \"typescript_handler\" but is missing handler.ts"
            );
        }

        capabilities.push(cap);
        permissions.push(perm);
    }

    Ok((capabilities, permissions))
}

fn load_typescript_tools_manifest(root: &Path) -> Result<HandlerManifest> {
    let tools_path = root.join("capabilities/handlers/tools.json");
    if !tools_path.is_file() {
        return Ok(HandlerManifest::new(Vec::new()));
    }
    let manifest = HandlerManifest::from_json_slice(
        &fs::read(&tools_path)
            .with_context(|| format!("Failed to read {}", tools_path.display()))?,
    )
    .with_context(|| format!("Failed to parse {}", tools_path.display()))?;
    manifest
        .validate()
        .with_context(|| format!("Invalid {}", tools_path.display()))?;
    Ok(manifest)
}

fn enrich_typescript_capabilities_from_tools_manifest(
    capabilities: &mut [CapabilityEntry],
    manifest: &HandlerManifest,
) -> Result<()> {
    let by_name: BTreeMap<&str, _> = manifest
        .handlers
        .iter()
        .filter(|entry| {
            entry.kind == HandlerKind::Tool && entry.language == HandlerLanguage::TypeScript
        })
        .map(|entry| (entry.name.as_str(), entry))
        .collect();

    for cap in capabilities {
        if capability_kind(cap) != Some("typescript_handler") {
            continue;
        }
        let Some(entry) = by_name.get(cap.id.as_str()) else {
            bail!(
                "capability '{}' has kind = \"typescript_handler\" but no matching \
                 tools.json entry named '{}'; run 'apxm agent build' or fix handler.ts",
                cap.id,
                cap.id
            );
        };
        let module = &entry.module;
        let qualname = &entry.qualname;
        cap.extra.insert(
            "handler_module".to_string(),
            toml::Value::String(module.to_string()),
        );
        cap.extra.insert(
            "handler_function".to_string(),
            toml::Value::String(qualname.to_string()),
        );
    }
    Ok(())
}

fn scan_skill_ids(root: &Path) -> Result<Vec<String>> {
    let skills_dir = root.join("skills");
    if !skills_dir.is_dir() {
        return Ok(Vec::new());
    }
    let mut out = Vec::new();
    let mut entries: Vec<_> = fs::read_dir(&skills_dir)
        .with_context(|| format!("Failed to read {}", skills_dir.display()))?
        .collect::<std::io::Result<Vec<_>>>()?;
    entries.sort_by_key(|entry| entry.path());
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
        let skill: SkillToml = read_toml(&skill_toml)?;
        out.push(skill.id);
    }
    Ok(out)
}

fn write_generated_capabilities_toml(path: &Path, capabilities: &[CapabilityEntry]) -> Result<()> {
    let mut lines = vec![SYNC_GENERATED_HEADER.to_string()];
    for cap in capabilities {
        lines.push("[[capability]]".to_string());
        lines.push(format!("id = \"{}\"", cap.id));
        if let Some(description) = &cap.description {
            lines.push(format!(
                "description = \"{}\"",
                description.replace('\\', "\\\\").replace('"', "\\\"")
            ));
        }
        for (key, value) in &cap.extra {
            lines.push(format!("{key} = {}", format_toml_value(value)));
        }
        lines.push(String::new());
    }
    fs::write(path, lines.join("\n")).with_context(|| format!("Failed to write {}", path.display()))
}

fn write_generated_permissions_toml(path: &Path, permissions: &[PermissionEntry]) -> Result<()> {
    let mut lines = vec![SYNC_GENERATED_HEADER.to_string()];
    for perm in permissions {
        lines.push("[[permission]]".to_string());
        lines.push(format!("capability = \"{}\"", perm.capability));
        if let Some(decision) = &perm.decision {
            lines.push(format!("decision = \"{decision}\""));
        }
        for (key, value) in &perm.extra {
            lines.push(format!("{key} = {}", format_toml_value(value)));
        }
        lines.push(String::new());
    }
    fs::write(path, lines.join("\n")).with_context(|| format!("Failed to write {}", path.display()))
}

fn format_toml_value(value: &toml::Value) -> String {
    match value {
        toml::Value::String(s) => format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\"")),
        toml::Value::Integer(i) => i.to_string(),
        toml::Value::Float(f) => f.to_string(),
        toml::Value::Boolean(b) => b.to_string(),
        toml::Value::Array(arr) => {
            let items: Vec<String> = arr.iter().map(format_toml_value).collect();
            format!("[{}]", items.join(", "))
        }
        other => other.to_string(),
    }
}

fn update_agent_inventories(root: &Path, capabilities: &[String], skills: &[String]) -> Result<()> {
    let path = root.join("agent.toml");
    let text =
        fs::read_to_string(&path).with_context(|| format!("Failed to read {}", path.display()))?;
    let mut doc: toml::Value =
        toml::from_str(&text).with_context(|| format!("Failed to parse {}", path.display()))?;
    let Some(table) = doc.as_table_mut() else {
        bail!("agent.toml must be a TOML table");
    };
    table.insert(
        "capabilities".into(),
        toml::Value::Array(
            capabilities
                .iter()
                .map(|cap| toml::Value::String(cap.clone()))
                .collect(),
        ),
    );
    table.insert(
        "skills".into(),
        toml::Value::Array(
            skills
                .iter()
                .map(|skill| toml::Value::String(skill.clone()))
                .collect(),
        ),
    );
    fs::write(
        &path,
        toml::to_string_pretty(&doc).context("Failed to serialize agent.toml")?,
    )
    .with_context(|| format!("Failed to write {}", path.display()))
}

pub(crate) fn agent_sync(root: &Path, json_output: bool) -> Result<()> {
    if !root.is_dir() {
        bail!("'{}' is not a directory", root.display());
    }
    let agent_path = root.join("agent.toml");
    if !agent_path.is_file() {
        bail!("missing required file: {}", agent_path.display());
    }
    let agent: AgentToml = read_toml(&agent_path)?;

    let (mut capabilities, permissions) = scan_capability_folders(root, &agent.id)?;
    compile_agent_handlers(root)?;
    let tools_manifest = load_typescript_tools_manifest(root)?;
    enrich_typescript_capabilities_from_tools_manifest(&mut capabilities, &tools_manifest)?;
    let capability_ids: Vec<String> = capabilities.iter().map(|cap| cap.id.clone()).collect();
    let skill_ids = scan_skill_ids(root)?;

    fs::create_dir_all(root.join("capabilities"))
        .with_context(|| format!("Failed to create {}", root.join("capabilities").display()))?;
    write_generated_capabilities_toml(&root.join("capabilities/capabilities.toml"), &capabilities)?;
    write_generated_permissions_toml(&root.join("capabilities/permissions.toml"), &permissions)?;
    update_agent_inventories(root, &capability_ids, &skill_ids)?;

    if json_output {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "path": root.display().to_string(),
                "capabilities": capability_ids,
                "skills": skill_ids,
                "status": "synced",
            }))?
        );
    } else {
        print_section_header("Agent Sync");
        print_status_line(
            &agent.id,
            Status::Ok,
            &format!(
                "{} capabilities, {} skills regenerated",
                capability_ids.len(),
                skill_ids.len()
            ),
        );
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

struct LoadedAgent {
    root: PathBuf,
    agent: AgentToml,
    hierarchy: Option<HierarchyToml>,
    capabilities: CapabilitiesToml,
    permissions: PermissionsToml,
    skills: Vec<SkillToml>,
}

fn load_agent(root: &Path) -> Result<LoadedAgent> {
    if !root.is_dir() {
        bail!("'{}' is not a directory", root.display());
    }
    let agent_path = root.join("agent.toml");
    if !agent_path.is_file() {
        bail!("missing required file: {}", agent_path.display());
    }
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

    Ok(LoadedAgent {
        root: root.to_path_buf(),
        agent,
        hierarchy,
        capabilities,
        permissions,
        skills,
    })
}

// ---------------------------------------------------------------------
// agent lint
// ---------------------------------------------------------------------

/// Every path recognized by `agent.v1#/properties/files` (see the
/// JSON schema's `patternProperties`). Anything else fails validation ("no
/// agent-private layout").
fn recognized_relpath(rel: &str) -> bool {
    if matches!(
        rel,
        "agent.toml"
            | "integrity.toml"
            | "hierarchy.toml"
            | "README.md"
            | "package.json"
            | "tsconfig.json"
    ) {
        return true;
    }
    let parts: Vec<&str> = rel.split('/').collect();
    match parts.as_slice() {
        ["capabilities", "capabilities.toml" | "permissions.toml"] => true,
        ["capabilities", "handlers", f] => {
            f.ends_with(".py") || f.ends_with(".ts") || *f == "tools.json"
        }
        [
            "capabilities",
            id,
            "capability.toml" | "permission.toml" | "handler.ts",
        ] if *id != "handlers" => true,
        ["prompts", f] => f.ends_with(".md"),
        ["python", f] => f.ends_with(".py"),
        ["skills", _id, "skill.toml"] => true,
        // SKILL.md / prompt.md are the primary prose; a skill may also carry
        // supplementary authored `.md` docs (e.g. a canvas/contract reference)
        // alongside them, plus its optional compiled artifacts and — for a
        // `compiled = true` skill — the single frontend source file
        // (`skill.py` or `skill.ts`) that emits its `skill.air`.
        ["skills", _id, f] => {
            f.ends_with(".md")
                || *f == "skill.air"
                || *f == "skill.apxmobj"
                || *f == "skill.py"
                || *f == "skill.ts"
        }
        ["skills", _id, "examples", f] => f.ends_with(".air"),
        ["examples", f] => f.ends_with(".md"),
        ["tests", f] => !f.is_empty(),
        ["shared", f] => !f.is_empty(),
        _ => false,
    }
}

/// Walk the agent root and return every recognized file as a
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
        if rel_str != "integrity.toml" && recognized_relpath(&rel_str) {
            out.push((rel_str, entry.path().to_path_buf()));
        }
    }
    out.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(out)
}

/// Find any file under the agent root that is *not* recognized by the
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

/// Load the org-global joined capability set (declared ∩ permitted, same
/// join rule used at the agent level and
/// `check_global_capability_join` uses at the org level) from an org
/// agent's own `capabilities/capabilities.toml` +
/// `capabilities/permissions.toml`, so agent lint validates
/// every skill's capability references against the agent's joined
/// capability set and the org-global joined set.
///
/// Missing files are treated as an empty global set (no org context, or an
/// org agent that declares no globals) rather than an error — this
/// function is opt-in plumbing for a agent that is a member of an org,
/// not a requirement every agent must satisfy.
fn load_org_global_capabilities(org_root: &Path) -> Result<BTreeSet<String>> {
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
        .map(|s| s.to_string())
        .collect())
}

/// The  capability-set agreement check: agent.toml's declared
/// capabilities must resolve into `capabilities.toml`, every
/// `capabilities.toml` entry must have a matching `permissions.toml` entry
/// (the "joined capability" — otherwise it "is not a capability and fails
/// lint"), every `agent.toml` skill reference must be a real `skills/<id>/`
/// directory, and every skill's own declared capabilities must be in the
/// agent's joined set (or, per , in `org_globals` — an org-global
/// capability is real without needing an agent-local permissions.toml
/// entry; the org agent that owns it already joined it at the org level).
/// Returns human-readable error strings; empty = clean.
fn check_capability_drift(pkg: &LoadedAgent, org_globals: &BTreeSet<String>) -> Vec<String> {
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
                 policy is not a capability"
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

    // agent.toml capabilities must resolve into the joined set, or be an
    // org-global capability (already joined at the org level — ).
    for cap in &pkg.agent.capabilities {
        if org_globals.contains(cap.as_str()) {
            continue;
        }
        if !declared_caps.contains(cap.as_str()) {
            errors.push(format!(
                "agent.toml declares capability '{cap}' which is not defined in \
                 capabilities/capabilities.toml (and is not an org-global capability)"
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

    // Every skill's own capability references must be inside the joined set,
    // or be an org-global capability — a skill invoking an
    // org-global capability must not be falsely flagged as undeclared.
    for skill in &pkg.skills {
        for cap in &skill.capabilities {
            if !joined_caps.contains(cap.as_str()) && !org_globals.contains(cap.as_str()) {
                errors.push(format!(
                    "skill '{}' invokes capability '{cap}' which is not in the agent's joined \
                     capability set (capabilities.toml + permissions.toml) and is not an \
                     org-global capability",
                    skill.id
                ));
            }
        }
    }

    errors
}

/// Structural + required-field checks for `agent.v1`: id/version, source
/// declaration, runtime loop, hierarchy, skills, and hook vocabulary.
fn check_schema_shape(pkg: &LoadedAgent) -> Vec<String> {
    let mut errors = Vec::new();

    if pkg.agent.id.trim().is_empty() {
        errors.push("agent.toml: id must not be empty".to_string());
    }
    if !semver_like(&pkg.agent.version) {
        errors.push(format!(
            "agent.toml: version '{}' is not valid SemVer",
            pkg.agent.version
        ));
    }
    match pkg.agent.schema_version.as_deref() {
        Some(AGENT_SCHEMA_V1) => {}
        Some(other) => errors.push(format!(
            "agent.toml: schema_version '{other}' must be '{AGENT_SCHEMA_V1}'"
        )),
        None => errors.push(format!(
            "agent.toml: schema_version is required and must be '{AGENT_SCHEMA_V1}'"
        )),
    }
    match declared_compile_source(&pkg.agent) {
        Ok(None) => {
            if pkg
                .agent
                .runtime
                .as_ref()
                .is_none_or(|runtime| !runtime_has_loop(runtime))
            {
                errors.push(
                    "agent.toml: an entry-less declarative package requires [runtime] loop"
                        .to_string(),
                );
            }
        }
        Ok(Some((entry, frontend))) => {
            let entry_path = Path::new(entry);
            if entry_path.is_absolute()
                || entry_path
                    .components()
                    .any(|component| matches!(component, std::path::Component::ParentDir))
            {
                errors.push(format!(
                    "agent.toml: [compile].entry '{entry}' must be a package-relative path"
                ));
            } else {
                let resolved = pkg.root.join(entry_path);
                let expected_extension = frontend.source_extension();
                if !entry.ends_with(expected_extension) {
                    errors.push(format!(
                        "agent.toml: [compile].entry '{entry}' must end in {expected_extension} for frontend '{frontend}'"
                    ));
                }
                if !resolved.is_file() {
                    errors.push(format!(
                        "agent.toml: [compile].entry '{entry}' does not exist at {}",
                        resolved.display()
                    ));
                }
            }
        }
        Err(error) => errors.push(error),
    }
    if let Some(runtime) = &pkg.agent.runtime {
        if let Some(message) = validate_runtime_loop(runtime) {
            errors.push(message);
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
            errors.push(format!(
                "agent.toml: hook event '{}' is not a recognized lifecycle event",
                hook.event
            ));
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
            if !skill_dir.join("skill.air").is_file() && !skill_dir.join("skill.apxmobj").is_file()
            {
                errors.push(format!(
                    "skill '{}' declares compiled = true but has neither skill.air nor skill.apxmobj",
                    skill.id
                ));
            }
            // a compiled skill must resolve to a real frontend source
            // file (`skill.py`/`skill.ts` per its declared `frontend`) — the
            // thing `agent build` actually recompiles.
            if let Err(err) = skill_source_path(&pkg.root, skill) {
                errors.push(err.to_string());
            }
        }
    }

    errors
}

/// Read the canonical package source declaration from `[compile]`.
fn declared_compile_source(agent: &AgentToml) -> Result<Option<(&str, FrontendLanguage)>, String> {
    let Some(compile) = agent.compile.as_ref() else {
        return Ok(None);
    };
    match (compile.entry.as_deref(), compile.frontend) {
        (None, None) => Ok(None),
        (Some(_), None) => Err(
            "agent.toml: [compile].entry requires [compile].frontend".to_string(),
        ),
        (None, Some(_)) => Err(
            "agent.toml: [compile].frontend requires [compile].entry; remove [compile] for an entry-less declarative package"
                .to_string(),
        ),
        (Some(entry), Some(frontend)) => Ok(Some((entry, frontend))),
    }
}

/// A `@hook(...)` call site found by statically scanning the agent's
/// `python/` tree.
struct ProgrammaticHook {
    file: String,
    line: usize,
    event: Option<String>,
    r#match: Option<String>,
    mode: Option<String>,
}

/// Extract `key="value"`/`key='value'` from a raw `@hook(...)` argument
/// string. Best-effort: only literal-string keyword arguments are
/// recognized (a hook that computes its event/mode dynamically is not
/// statically checkable and is skipped, not flagged — this lint fails
/// closed on *detectable* contradictions, it does not claim to prove their
/// absence).
fn extract_kwarg(args: &str, key: &str) -> Option<String> {
    // Accept `key = "value"` / `key='value'`.
    let needle = key;
    let mut search_from = 0;
    while let Some(rel) = args[search_from..].find(&needle) {
        let pos = search_from + rel;
        // Ensure this is a whole keyword token, not a substring of another
        // identifier (e.g. "match" inside "rematch").
        let boundary_ok = pos == 0
            || !args.as_bytes()[pos - 1].is_ascii_alphanumeric()
                && args.as_bytes()[pos - 1] != b'_';
        if !boundary_ok {
            search_from = pos + needle.len();
            continue;
        }
        let rest = args[pos + needle.len()..].trim_start();
        if let Some(rest) = rest.strip_prefix('=') {
            let rest = rest.trim_start();
            for quote in ['"', '\''] {
                if let Some(rest) = rest.strip_prefix(quote) {
                    if let Some(end) = rest.find(quote) {
                        return Some(rest[..end].to_string());
                    }
                }
            }
        }
        search_from = pos + needle.len();
    }
    None
}

/// Statically scan every `.py` file under `python/` for `@hook(on=…,
/// match=…, mode=…)` call sites ( lint: manifest/entry contradiction
/// check). This is a textual scan, not a Python parse/AST — it is
/// deliberately conservative (see [`extract_kwarg`]) rather than a full
/// interpreter, matching the rest of this module's "hand-port the schema's
/// checks without vendoring a runtime" approach.
fn find_programmatic_hooks(python_dir: &Path) -> Vec<ProgrammaticHook> {
    let mut hooks = Vec::new();
    if !python_dir.is_dir() {
        return hooks;
    }
    for entry in walkdir::WalkDir::new(python_dir)
        .into_iter()
        .filter_map(Result::ok)
    {
        if !entry.file_type().is_file() {
            continue;
        }
        if entry.path().extension().and_then(|e| e.to_str()) != Some("py") {
            continue;
        }
        let Ok(text) = fs::read_to_string(entry.path()) else {
            continue;
        };
        let rel = entry
            .path()
            .strip_prefix(python_dir)
            .unwrap_or(entry.path())
            .display()
            .to_string();
        let mut search_from = 0usize;
        while let Some(rel_idx) = text[search_from..].find("@hook(") {
            let start = search_from + rel_idx;
            let args_start = start + "@hook(".len();
            let Some(close_rel) = text[args_start..].find(')') else {
                break;
            };
            let args = &text[args_start..args_start + close_rel];
            let line = text[..start].matches('\n').count() + 1;
            hooks.push(ProgrammaticHook {
                file: rel.clone(),
                line,
                event: extract_kwarg(args, "on"),
                r#match: extract_kwarg(args, "match"),
                mode: extract_kwarg(args, "mode"),
            });
            search_from = args_start + close_rel + 1;
        }
    }
    hooks
}

/// entry code may add hooks programmatically (`@hook`) but must never
/// *contradict* what `agent.toml`'s `[[hooks]]` already declares for the
/// same `(event, match)` pair — a different `mode` there is a silent
/// footgun (an author reads the manifest and gets the entry's behavior
/// instead), so it is a lint error, not a warning.
fn check_hook_contradictions(pkg: &LoadedAgent) -> Vec<String> {
    let mut errors = Vec::new();
    let python_dir = pkg.root.join("python");
    let programmatic = find_programmatic_hooks(&python_dir);

    for declared in &pkg.agent.hooks {
        let declared_match = declared.r#match.clone().unwrap_or_else(|| "*".to_string());
        for found in &programmatic {
            let (Some(event), Some(mode)) = (found.event.as_deref(), found.mode.as_deref()) else {
                continue;
            };
            if event != declared.event {
                continue;
            }
            let found_match = found.r#match.clone().unwrap_or_else(|| "*".to_string());
            if found_match != declared_match {
                continue;
            }
            if mode != declared.mode {
                errors.push(format!(
                    "agent.toml declares hook on event '{}' match '{}' with mode '{}', but \
                     python/{}:{} registers @hook(on=\"{event}\", match=\"{found_match}\", \
                     mode=\"{mode}\") — entry code must not contradict the manifest",
                    declared.event, declared_match, declared.mode, found.file, found.line
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
    parts.len() == 3
        && parts
            .iter()
            .all(|p| !p.is_empty() && p.chars().all(|c| c.is_ascii_digit()))
}

/// Validate capability implementation metadata against the canonical server
/// loader contract. Builtin dispatch uses `id`; grouped builtins use
/// `builtin_group`; `runtime` and `binding` are rejected.
fn check_capability_bindings(pkg: &LoadedAgent) -> Vec<String> {
    use apxm_ais::capabilities::{BUILTIN_GROUPS, BUILTINS};
    let mut errors = Vec::new();
    for cap in &pkg.capabilities.capability {
        let kind = capability_kind(cap);
        if cap.extra.contains_key("binding") {
            errors.push(format!(
                "capability '{}' uses removed field 'binding'; use id for builtin dispatch \
                 and builtin_group for grouped builtins",
                cap.id
            ));
        }
        match kind {
            Some("builtin") => {
                let builtin_group = cap.extra.get("builtin_group").and_then(|v| v.as_str());
                if BUILTINS.contains(&cap.id.as_str()) {
                    continue;
                }
                match builtin_group {
                    Some(group) if BUILTIN_GROUPS.contains(&group) => {}
                    Some(group) => errors.push(format!(
                        "capability '{}' (kind=builtin) has unknown builtin_group '{group}'; expected one of {BUILTIN_GROUPS:?}",
                        cap.id
                    )),
                    None => errors.push(format!(
                        "capability '{}' (kind=builtin) is not a registered builtin id and is missing builtin_group",
                        cap.id
                    )),
                }
            }
            Some("typescript_handler" | "python_handler") => {
                if !cap.extra.contains_key("handler_module") {
                    errors.push(format!(
                        "capability '{}' (kind={}) is missing handler_module",
                        cap.id,
                        kind.unwrap()
                    ));
                }
                if !cap.extra.contains_key("handler_function") {
                    errors.push(format!(
                        "capability '{}' (kind={}) is missing handler_function",
                        cap.id,
                        kind.unwrap()
                    ));
                }
            }
            Some("host" | "provider" | "http" | "static" | "mcp") => {}
            Some(other) => errors.push(format!(
                "capability '{}' has unsupported kind '{other}'; expected one of \
                 builtin, typescript_handler, python_handler, host, provider, http, static, mcp",
                cap.id
            )),
            None => {
                errors.push(format!("capability '{}' is missing kind", cap.id));
            }
        }
    }
    errors
}

pub(crate) fn agent_lint(path: &Path, org: Option<PathBuf>, json_output: bool) -> Result<()> {
    let pkg = load_agent(path)?;
    let org_globals = match &org {
        Some(org_root) => load_org_global_capabilities(org_root)
            .with_context(|| format!("Failed to load org globals from {}", org_root.display()))?,
        None => BTreeSet::new(),
    };

    let mut errors = check_schema_shape(&pkg);
    errors.extend(check_capability_drift(&pkg, &org_globals));
    errors.extend(check_hook_contradictions(&pkg));
    errors.extend(check_capability_bindings(&pkg));
    // a hand-edited compiled artifact is a lint error in every
    // dialect. This is the "lighter-weight check" documented on
    // `detect_hand_edited_artifact` — comparing recorded vs current hashes,
    // not recompiling — which is cheap enough to run unconditionally (lint
    // has no `driver`-feature dependency, unlike `agent build`'s
    // recompilation step).
    for skill in &pkg.skills {
        if let Some(message) = detect_hand_edited_artifact(path, skill)? {
            errors.push(message);
        }
    }
    for unrecognized in find_unrecognized_files(path)? {
        errors.push(format!(
            "unrecognized file '{unrecognized}' is not part of the agent.v1 folder contract"
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
        print_section_header("Agent Lint");
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
// agent build — integrity hash chain ( `integrity` schema)
// ---------------------------------------------------------------------

fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    let digest = hasher.finalize();
    digest.iter().map(|b| format!("{b:02x}")).collect()
}

/// Genesis `prev_hash` for the first chain link — 64 zeros, per .
const GENESIS_HASH: &str = "0000000000000000000000000000000000000000000000000000000000000000";
const _GENESIS_HASH_IS_64_HEX_CHARS: () = assert!(GENESIS_HASH.len() == 64);

/// Compute the  integrity hash chain over `files` (relpath -> content
/// digest), in ascending relpath order. Each link's hash is
/// `sha256(prev_hash || path || file_digest)` (UTF-8 byte concatenation of
/// the two hex digests and the path string); `chain[0].prev_hash` is 64
/// zeros (genesis); `hash` is the final link's hash.
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
    let hash = chain
        .last()
        .map(|l| l.hash.clone())
        .unwrap_or_else(|| genesis.to_string());
    IntegrityToml {
        algorithm: "sha256".to_string(),
        hash,
        chain,
    }
}

/// Recompute a digest map from a agent root's currently-recognized files
/// (used both to build and to independently verify a hash chain). `agent.toml`
/// is hashed as raw authored bytes; generated `integrity.toml` is excluded.
fn digest_recognized_files(root: &Path) -> Result<BTreeMap<String, String>> {
    let mut files = BTreeMap::new();
    for (rel, abs) in walk_recognized_files(root)? {
        let bytes = fs::read(&abs).with_context(|| format!("Failed to read {}", abs.display()))?;
        files.insert(rel, sha256_hex(&bytes));
    }
    Ok(files)
}

fn write_integrity_toml(path: &Path, integrity: &IntegrityToml) -> Result<()> {
    let text = toml::to_string_pretty(integrity).context("Failed to serialize integrity.toml")?;
    fs::write(
        path,
        format!("# Generated by apxm agent build. Do not edit.\n{text}"),
    )
    .with_context(|| format!("Failed to write {}", path.display()))
}

/// Verify a built agent package against its generated integrity chain.
#[cfg(feature = "driver")]
pub(super) fn verify_agent_integrity(root: &Path) -> Result<()> {
    let integrity_path = root.join("integrity.toml");
    if !integrity_path.is_file() {
        bail!(
            "agent package '{}' is missing integrity.toml; run 'apxm agent build {}' before compiling it",
            root.display(),
            root.display()
        );
    }

    let recorded: IntegrityToml = read_toml(&integrity_path)?;
    if recorded.algorithm != "sha256" {
        bail!(
            "agent package '{}' uses unsupported integrity algorithm '{}'; expected 'sha256'",
            root.display(),
            recorded.algorithm
        );
    }

    let unrecognized = find_unrecognized_files(root)?;
    if !unrecognized.is_empty() {
        bail!(
            "agent package '{}' contains files outside the integrity schema: {}; move them into the declared package layout and run 'apxm agent build {}' again",
            root.display(),
            unrecognized.join(", "),
            root.display()
        );
    }

    let expected = compute_integrity(&digest_recognized_files(root)?);
    if recorded != expected {
        bail!(
            "agent package '{}' failed integrity verification; package contents changed after the last build, so run 'apxm agent build {}' again",
            root.display(),
            root.display()
        );
    }
    Ok(())
}

#[cfg(test)]
pub(super) fn seal_agent_integrity_for_test(root: &Path) -> Result<()> {
    let integrity = compute_integrity(&digest_recognized_files(root)?);
    write_integrity_toml(&root.join("integrity.toml"), &integrity)
}

/// Resolve a compiled skill's single frontend source file
/// (`skills/<id>/skill.py` or `skills/<id>/skill.ts`) from its declared
/// `frontend`. Errors clearly for an unknown `frontend` value or a missing
/// source file rather than silently falling back.
fn skill_source_path(root: &Path, skill: &SkillToml) -> Result<PathBuf> {
    let dir = root.join("skills").join(&skill.id);
    let frontend = skill.frontend.ok_or_else(|| {
        anyhow!(
            "skill '{}' declares compiled = true but does not declare frontend",
            skill.id
        )
    })?;
    let source_path = dir.join(frontend.skill_source_filename());
    if !source_path.is_file() {
        bail!(
            "skill '{}' declares compiled = true with frontend = \"{}\" but is missing {}",
            skill.id,
            frontend,
            source_path.display()
        );
    }
    Ok(source_path)
}

fn sha256_hex_file(path: &Path) -> Result<String> {
    let bytes = fs::read(path).with_context(|| format!("Failed to read {}", path.display()))?;
    Ok(sha256_hex(&bytes))
}

/// Hand-edited-artifact detection, shared by `agent lint` (a light,
/// metadata-only check — see the doc comment on `agent_lint`'s call site)
/// and `agent build` (a hard pre-compile gate — see `agent_build`).
///
/// Semantics: a `skill.air` whose hash doesn't match its recorded source is a
/// build error, but the check must be precise about which recorded hash and
/// which mismatch triggers it.
/// `skill.toml` remembers, from the last successful compiled build of a
/// skill, both `source_hash` (hash of the frontend source file compiled)
/// and `air_hash` (hash of the `skill.air` that compile produced). If the
/// *source* has not changed since that build (`source_hash` still matches
/// the source file on disk) but `skill.air`'s current bytes no longer match
/// the recorded `air_hash`, the only way that diff can have appeared is a
/// hand edit of the compiled artifact — recompiling the (unchanged) source
/// would reproduce the recorded `air_hash`, not what's on disk. That is a
/// sound, cheap (no recompilation required), and unambiguous drift signal.
///
/// A normal source edit (the common, expected `agent build` case) is
/// explicitly *not* flagged here: `source_hash` no longer matching current
/// source is exactly what recompiling is for, not an error condition. A
/// skill with no recorded `source_hash`/`air_hash` yet (first build, or a
/// skill.toml predating this field) is likewise not flagged — "no prior
/// recorded hash = normal first build, not an error".
fn detect_hand_edited_artifact(root: &Path, skill: &SkillToml) -> Result<Option<String>> {
    if !skill.compiled {
        return Ok(None);
    }
    let (Some(recorded_source_hash), Some(recorded_air_hash)) =
        (skill.source_hash.as_deref(), skill.air_hash.as_deref())
    else {
        return Ok(None);
    };
    let air_path = root.join("skills").join(&skill.id).join("skill.air");
    if !air_path.is_file() {
        return Ok(None);
    }
    let source_path = skill_source_path(root, skill)?;
    let current_source_hash = sha256_hex_file(&source_path)?;
    if current_source_hash != recorded_source_hash {
        // Source changed on purpose; recompiling (not an error) will bring
        // skill.air back in sync.
        return Ok(None);
    }
    let current_air_hash = sha256_hex_file(&air_path)?;
    if current_air_hash != recorded_air_hash {
        return Ok(Some(format!(
            "skill '{}': skill.air does not match its last recorded build, and the skill's \
             source ({}) is unchanged since that build — recompiling would reproduce the \
             recorded artifact, so this diff can only come from hand-editing skill.air. Edit \
             the source and run 'apxm agent build' again, or discard the hand edit.",
            skill.id,
            source_path.display()
        )));
    }
    Ok(None)
}

/// Compile one `compiled = true` skill's frontend source into `skill.air`,
/// then convert that AIR into `skill.apxmobj` via the same
/// `apxm_driver::compiler::Compiler` pipeline `apxm compile` drives.
/// Returns `(source_hash, air_hash)` for the caller to persist into the
/// skill's `skill.toml`.
#[cfg(feature = "driver")]
fn compile_skill(root: &Path, skill: &SkillToml) -> Result<(String, String)> {
    let source_path = skill_source_path(root, skill)?;
    let source_hash = sha256_hex_file(&source_path)?;

    let frontend = skill.frontend.ok_or_else(|| {
        anyhow!(
            "skill '{}' declares compiled = true but does not declare frontend",
            skill.id
        )
    })?;
    let (air_text, handler_manifest_data) = match frontend {
        FrontendLanguage::Python => {
            let (tmp, handler_manifest) = super::compile::emit_air_from_python(&source_path, None)
                .with_context(|| {
                    format!(
                        "Failed to compile skill '{}' from {}",
                        skill.id,
                        source_path.display()
                    )
                })?;
            (
                fs::read_to_string(tmp.path()).context("Failed to read emitted AIR")?,
                handler_manifest,
            )
        }
        FrontendLanguage::TypeScript => {
            let (tmp, handler_manifest) = super::compile::emit_air_from_typescript(&source_path)
                .with_context(|| {
                    format!(
                        "Failed to compile skill '{}' from {}",
                        skill.id,
                        source_path.display()
                    )
                })?;
            (
                fs::read_to_string(tmp.path()).context("Failed to read emitted AIR")?,
                handler_manifest,
            )
        }
    };

    let dir = root.join("skills").join(&skill.id);
    let air_path = dir.join("skill.air");
    fs::write(&air_path, air_text.as_bytes())
        .with_context(|| format!("Failed to write {}", air_path.display()))?;
    let air_hash = sha256_hex(air_text.as_bytes());

    let compiler = Compiler::with_opt_level(apxm_core::types::OptimizationLevel::O0)
        .context("Failed to initialize compiler (MLIR toolchain not detected)")?;
    let module = compiler
        .compile(&air_path)
        .map_err(|err| anyhow!("Failed to compile skill '{}' AIR: {err}", skill.id))?;
    let handler_manifest = handler_manifest_data
        .as_deref()
        .map(|data| {
            let manifest = HandlerManifest::from_json_slice(data)
                .context("Failed to parse compiled skill handler manifest")?;
            manifest
                .validate()
                .context("Invalid compiled skill handler manifest")?;
            Ok::<_, anyhow::Error>(manifest)
        })
        .transpose()?;
    let mut artifact = module
        .generate_artifact_with_manifest(None, handler_manifest.as_ref())
        .with_context(|| format!("Failed to generate artifact for skill '{}'", skill.id))?;
    if let Some(data) = handler_manifest_data {
        artifact.add_section(apxm_artifact::ArtifactSection {
            kind: apxm_core::types::HANDLER_MANIFEST_ARTIFACT_SECTION.into(),
            data,
        });
    }
    // Pin created_at so the wire bytes (and this build's air_hash-paired
    // apxmobj) are stable across rebuilds of unchanged source, same as
    // `compile.rs::compile_command`'s `--embed-manifest` path.
    artifact.set_created_at(0);
    let bytes = artifact
        .to_bytes()
        .map_err(|err| anyhow!("Failed to serialize skill '{}' artifact: {err}", skill.id))?;
    let obj_path = dir.join("skill.apxmobj");
    fs::write(&obj_path, &bytes)
        .with_context(|| format!("Failed to write {}", obj_path.display()))?;

    Ok((source_hash, air_hash))
}

#[cfg(not(feature = "driver"))]
fn compile_skill(_root: &Path, skill: &SkillToml) -> Result<(String, String)> {
    println!("Compiling skills requires the driver feature");
    println!("Rebuild with: {}", super::dekk_hints::BUILD);
    Err(anyhow!(
        "skill '{}' declares compiled = true; recompiling it requires the driver feature",
        skill.id
    ))
}

fn write_skill_build_hashes(
    root: &Path,
    skill_id: &str,
    source_hash: &str,
    air_hash: &str,
) -> Result<()> {
    let path = root.join("skills").join(skill_id).join("skill.toml");
    let mut skill: SkillToml = read_toml(&path)?;
    skill.source_hash = Some(source_hash.to_string());
    skill.air_hash = Some(air_hash.to_string());
    let text = toml::to_string_pretty(&skill).context("Failed to serialize skill.toml")?;
    fs::write(&path, text).with_context(|| format!("Failed to write {}", path.display()))
}

fn collect_typescript_handler_sources(root: &Path) -> Result<Vec<PathBuf>> {
    let mut sources = BTreeSet::new();
    let handlers_dir = root.join("capabilities/handlers");
    if handlers_dir.is_dir() {
        for entry in fs::read_dir(&handlers_dir)
            .with_context(|| format!("Failed to read {}", handlers_dir.display()))?
        {
            let path = entry?.path();
            if path.extension().and_then(|ext| ext.to_str()) == Some("ts") {
                sources.insert(path);
            }
        }
    }

    let caps_dir = root.join("capabilities");
    if caps_dir.is_dir() {
        for entry in fs::read_dir(&caps_dir)
            .with_context(|| format!("Failed to read {}", caps_dir.display()))?
        {
            let path = entry?.path();
            if !path.is_dir() {
                continue;
            }
            let cap_path = path.join("capability.toml");
            if !cap_path.is_file() {
                continue;
            }
            let cap: CapabilityEntry = read_toml(&cap_path)?;
            if capability_kind(&cap) == Some("typescript_handler") {
                let handler = path.join("handler.ts");
                if handler.is_file() {
                    sources.insert(handler);
                }
            }
        }
    }

    Ok(sources.into_iter().collect())
}

fn validate_typescript_handler_toolchain(ts_frontend: &Path) -> Result<()> {
    let node_modules = ts_frontend.join("node_modules");
    if !node_modules.join("esbuild").is_dir() {
        bail!(
            "TypeScript handler compiler dependencies are missing under {}; run `dekk agents frontend setup`",
            node_modules.display()
        );
    }
    let compiler = ts_frontend.join("dist/compile-handlers.js");
    if !compiler.is_file() {
        bail!(
            "TypeScript handler compiler is not built at {}; run `dekk agents frontend build`",
            compiler.display()
        );
    }
    Ok(())
}

fn compile_agent_handlers(root: &Path) -> Result<()> {
    let root = root.canonicalize().with_context(|| {
        format!(
            "Failed to resolve absolute path for agent root {}",
            root.display()
        )
    })?;
    let sources = collect_typescript_handler_sources(&root)?;
    if sources.is_empty() {
        return Ok(());
    }

    let out = root.join("capabilities/handlers/tools.json");
    fs::create_dir_all(out.parent().expect("tools.json has parent"))
        .with_context(|| format!("Failed to create {}", out.parent().unwrap().display()))?;

    let source_args: Vec<String> = sources
        .iter()
        .map(|source| source.to_string_lossy().into_owned())
        .collect();

    let compiler = installed_typescript_frontend_entry("dist/compile-handlers.js")?;
    let status = std::process::Command::new("node")
        .arg("--input-type=module")
        .arg("--eval")
        .arg(
            "import { pathToFileURL } from 'node:url'; import(pathToFileURL(process.argv[1]).href).then(async ({ compileHandlers }) => { const fs = await import('node:fs/promises'); const [out, root, ...sources] = process.argv.slice(2); const manifest = await compileHandlers(sources, { rootDir: root }); await fs.writeFile(out, `${JSON.stringify(manifest, null, 2)}\\n`); })",
        )
        .arg(&compiler)
        .arg(&out)
        .arg(&root)
        .args(&source_args)
        .status()
        .with_context(|| {
            format!(
                "Failed to run installed @apxm/frontend handler compiler {} (set {} to its package root)",
                compiler.display(),
                apxm_env::APXM_TYPESCRIPT_FRONTEND_PACKAGE,
            )
        })?;
    if !status.success() {
        bail!("installed @apxm/frontend handler compiler failed for agent handlers");
    }
    Ok(())
}

pub(crate) fn agent_build(path: &Path, json_output: bool) -> Result<()> {
    agent_sync(path, false)?;
    let pkg = load_agent(path)?;
    let mut compiled_skills: Vec<String> = Vec::new();

    for skill in &pkg.skills {
        if !skill.compiled {
            continue;
        }
        // Hard gate ( "hash-mismatch build error"): refuse to silently
        // clobber a hand-edited skill.air rather than compiling over it —
        // see `detect_hand_edited_artifact`'s doc comment for the exact
        // mismatch this catches.
        if let Some(message) = detect_hand_edited_artifact(path, skill)? {
            bail!(message);
        }
        let (source_hash, air_hash) = compile_skill(path, skill)?;
        write_skill_build_hashes(path, &skill.id, &source_hash, &air_hash)?;
        compiled_skills.push(skill.id.clone());
    }

    compile_agent_handlers(path)?;
    // i.e. the FINAL post-compilation artifacts (freshly written
    // skill.air/skill.apxmobj and the skill.toml files just updated with
    // source_hash/air_hash), not the pre-compilation source tree. agent.toml
    // itself is untouched by compiling skills.
    let files = digest_recognized_files(path)?;
    let integrity = compute_integrity(&files);

    write_integrity_toml(&path.join("integrity.toml"), &integrity)?;

    if json_output {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "path": path.display().to_string(),
                "files": files.len(),
                "hash": integrity.hash,
                "compiled_skills": compiled_skills,
            }))?
        );
    } else {
        print_section_header("Agent Build");
        print_status_line(
            &pkg.agent.id,
            Status::Ok,
            &format!("{} files hashed, hash={}", files.len(), integrity.hash),
        );
        if !compiled_skills.is_empty() {
            println!("  compiled skills: {}", compiled_skills.join(", "));
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------
// agent install
// ---------------------------------------------------------------------

pub(crate) fn agents_dir(apxm_home: &Path) -> PathBuf {
    apxm_home.join("agents")
}

/// Resolve an installed agent's `agent.toml` under
/// `APXM_HOME/agents/<id>/` — the same install destination
/// [`agent_install_to`] writes to. Shared with `org lint`'s member
/// resolution check so org-agent member references are checked
/// against the same install layout `agent install` created,
/// rather than a second hand-rolled resolution path.
pub(crate) fn resolve_installed_agent(apxm_home: &Path, id: &str) -> Result<AgentToml> {
    let pack_path = agents_dir(apxm_home).join(id).join("agent.toml");
    if !pack_path.is_file() {
        bail!(
            "agent agent '{id}' is not installed under {} (run 'apxm agent install' first)",
            agents_dir(apxm_home).display()
        );
    }
    read_toml(&pack_path)
}

/// Recursively copy a directory tree. Shared by `agent install` and
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
                format!(
                    "Failed to copy {} to {}",
                    src_path.display(),
                    dst_path.display()
                )
            })?;
        }
    }
    Ok(())
}

fn agent_install(path: &Path, force: bool, json_output: bool) -> Result<()> {
    agent_install_to(path, &apxm_core::env::apxm_home(), force, json_output)
}

/// Installs a agent under an explicit `apxm_home` root. Split out from
/// [`agent_install`] so tests can point at a tempdir instead of mutating
/// the process-global `APXM_HOME` env var (this crate denies `unsafe_code`,
/// which `std::env::set_var` requires in Rust 2024).
pub(crate) fn agent_install_to(
    path: &Path,
    apxm_home: &Path,
    force: bool,
    json_output: bool,
) -> Result<()> {
    if !path.join("integrity.toml").is_file() {
        agent_build(path, false)?;
    }
    let pkg = load_agent(path)?;
    let dest = agents_dir(apxm_home).join(&pkg.agent.id);

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
        .with_context(|| format!("Failed to create {}", agents_dir(apxm_home).display()))?;
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
        print_section_header("Agent Installed");
        print_status_line(&pkg.agent.id, Status::Ok, &dest.display().to_string());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn scaffold(dir: &Path, id: &str) {
        agent_new(id, Some(dir.to_path_buf()), None, "looped-agent", true).expect("scaffold ok");
    }

    #[test]
    fn new_scaffolds_schema_valid_tree() {
        let tmp = tempdir().unwrap();
        let root = tmp.path().join("demo");
        scaffold(&root, "demo");

        for rel in [
            "agent.toml",
            "hierarchy.toml",
            "capabilities/capabilities.toml",
            "capabilities/permissions.toml",
            "capabilities/read/capability.toml",
            "capabilities/write/capability.toml",
            "capabilities/handlers/hooks.ts",
            "package.json",
            "tsconfig.json",
            "prompts/persona.md",
            "skills/demo-skill/skill.toml",
            "skills/demo-skill/SKILL.md",
            "skills/demo-skill/prompt.md",
            "examples/basic.md",
            "tests/README.md",
        ] {
            assert!(root.join(rel).is_file(), "missing scaffolded file: {rel}");
        }

        let agent: AgentToml = read_toml(&root.join("agent.toml")).unwrap();
        assert_eq!(agent.id, "demo");
        assert_eq!(agent.version, "0.1.0");
        assert_eq!(agent.schema_version.as_deref(), Some(AGENT_SCHEMA_V1));
        assert!(agent.compile.is_none());

        assert_eq!(agent.kind.as_deref(), Some("agent"));
        assert_eq!(agent.capabilities, vec!["read", "write"]);
        assert_eq!(agent.skills, vec!["demo-skill".to_string()]);
        match agent
            .runtime
            .as_ref()
            .and_then(|runtime| runtime.r#loop.as_ref())
        {
            Some(LoopToml::Config(table)) => {
                assert_eq!(table.mode, "recv");
                assert!(table.rearm);
                assert_eq!(table.turn_param, "user_message");
            }
            other => panic!("expected [runtime.loop] table, got {other:?}"),
        }

        let skill: SkillToml = read_toml(&root.join("skills/demo-skill/skill.toml")).unwrap();
        assert_eq!(skill.id, "demo-skill");
        assert!(!skill.compiled);

        // The freshly scaffolded tree must lint clean (no capability
        // declared => nothing to join, no drift).
        agent_lint(&root, None, true).expect("scaffolded agent should lint clean");
    }

    #[test]
    fn lint_rejects_missing_schema_version() {
        let tmp = tempdir().unwrap();
        let root = tmp.path().join("demo");
        scaffold(&root, "demo");
        let agent_path = root.join("agent.toml");
        let text = fs::read_to_string(&agent_path).unwrap();
        fs::write(
            &agent_path,
            text.replace("schema_version = \"apxm.agent.v1\"\n", ""),
        )
        .unwrap();

        let pkg = load_agent(&root).unwrap();
        let errors = check_schema_shape(&pkg);
        assert!(
            errors
                .iter()
                .any(|error| error.contains("schema_version is required")),
            "{errors:?}"
        );
        agent_lint(&root, None, true).expect_err("missing schema_version must fail");
    }

    #[test]
    fn agent_toml_rejects_unsupported_top_level_entry() {
        let error = toml::from_str::<AgentToml>(
            "id = \"demo\"\nversion = \"0.1.0\"\nentry = \"python/main.py\"\n",
        )
        .expect_err("top-level entry is not part of the agent manifest");
        assert!(error.to_string().contains("unknown field `entry`"));
    }

    #[test]
    fn new_refuses_nonempty_destination() {
        let tmp = tempdir().unwrap();
        let root = tmp.path().join("demo");
        scaffold(&root, "demo");
        let err = agent_new("demo", Some(root.clone()), None, "looped-agent", true).unwrap_err();
        assert!(err.to_string().contains("already exists"));
    }

    #[test]
    fn lint_passes_on_consistent_agent() {
        let tmp = tempdir().unwrap();
        let root = tmp.path().join("clean");
        scaffold(&root, "clean");

        // Add one joined capability referenced by agent.toml and used by the skill.
        fs::create_dir_all(root.join("capabilities/clean_example")).unwrap();
        fs::write(
            root.join("capabilities/clean_example/capability.toml"),
            "id = \"clean_example\"\ndescription = \"demo\"\nkind = \"static\"\n",
        )
        .unwrap();
        fs::write(
            root.join("capabilities/clean_example/permission.toml"),
            "capability = \"clean_example\"\ndecision = \"allow\"\n",
        )
        .unwrap();
        agent_sync(&root, true).unwrap();
        fs::write(
            root.join("skills/clean-skill/skill.toml"),
            "id = \"clean-skill\"\ncompiled = false\ncapabilities = [\"clean_example\"]\n",
        )
        .unwrap();

        agent_lint(&root, None, true).expect("consistent agent should lint clean");
    }

    #[test]
    fn lint_catches_capability_set_drift() {
        let tmp = tempdir().unwrap();
        let root = tmp.path().join("drift");
        scaffold(&root, "drift");

        // agent.toml references a capability that capabilities.toml never declares.
        update_agent_inventories(
            &root,
            &["drift.undeclared".to_string()],
            &["drift-skill".to_string()],
        )
        .unwrap();

        let err = agent_lint(&root, None, true).expect_err("drift must fail lint");
        assert!(err.to_string().contains("lint error"));

        // Now declare the capability but withhold its permissions entry —
        // "not a capability" per .
        fs::write(
            root.join("capabilities/capabilities.toml"),
            "[[capability]]\nid = \"drift.undeclared\"\n",
        )
        .unwrap();
        let err =
            agent_lint(&root, None, true).expect_err("missing permission entry must fail lint");
        assert!(
            err.to_string().contains("lint error"),
            "expected a lint error, got: {err}"
        );
    }

    #[test]
    fn new_resolves_example_names_and_template_paths_generically() {
        let gao = resolve_agent_template_dir("gao").expect("named example resolves");
        assert_eq!(gao, example_agent_dir("gao"));

        let tmp = tempdir().unwrap();
        let template = tmp.path().join("custom-template");
        fs::create_dir_all(&template).unwrap();
        fs::write(
            template.join("agent.toml"),
            "id = \"template\"\nversion = \"0.1.0\"\n\n[runtime]\nsession_prefix = \"template\"\n",
        )
        .unwrap();
        let destination = tmp.path().join("generated");
        agent_new(
            "generated",
            Some(destination.clone()),
            Some("Generated Agent".to_string()),
            template.to_str().expect("UTF-8 path"),
            true,
        )
        .expect("path template scaffolds");

        let agent: AgentToml = read_toml(&destination.join("agent.toml")).unwrap();
        assert_eq!(agent.id, "generated");
        assert_eq!(agent.display_name.as_deref(), Some("Generated Agent"));
        assert_eq!(
            agent
                .runtime
                .as_ref()
                .and_then(|runtime| runtime.session_prefix.as_deref()),
            Some("generated")
        );
        assert!(!destination.join("integrity.toml").exists());
    }

    #[test]
    fn typescript_manifest_joins_permission_decisions_into_approval_metadata() {
        let tmp = tempdir().unwrap();
        let root = tmp.path();
        fs::create_dir_all(root.join("capabilities/handlers")).unwrap();
        fs::write(
            root.join("capabilities/handlers/tools.json"),
            r#"[
                {"handler_id":"sha256:read","module":"read","qualname":"run","name":"read_tool","schema":{}},
                {"handler_id":"sha256:write","module":"write","qualname":"run","name":"write_tool","schema":{}}
            ]"#,
        )
        .unwrap();
        let capability = |id: &str, read_only: bool| CapabilityEntry {
            id: id.to_string(),
            description: None,
            extra: toml::Table::from_iter([
                (
                    "kind".to_string(),
                    toml::Value::String("typescript_handler".to_string()),
                ),
                ("read_only".to_string(), toml::Value::Boolean(read_only)),
            ]),
        };
        let permission = |id: &str, decision: &str| PermissionEntry {
            capability: id.to_string(),
            decision: Some(decision.to_string()),
            extra: toml::Table::new(),
        };

        annotate_typescript_tools_manifest(
            root,
            &[
                capability("read_tool", true),
                capability("write_tool", false),
            ],
            &[
                permission("read_tool", "allow"),
                permission("write_tool", "ask"),
            ],
        )
        .expect("manifest annotation");

        let manifest = load_typescript_tools_manifest(root).unwrap();
        let by_name = manifest
            .iter()
            .map(|entry| (entry["name"].as_str().unwrap(), entry))
            .collect::<BTreeMap<_, _>>();
        assert_eq!(by_name["read_tool"]["read_only"], true);
        assert_eq!(by_name["read_tool"]["requires_approval"], false);
        assert_eq!(by_name["write_tool"]["read_only"], false);
        assert_eq!(by_name["write_tool"]["requires_approval"], true);
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
        let err =
            agent_lint(&root, None, true).expect_err("undeclared skill capability must fail lint");
        assert!(err.to_string().contains("lint error"));
    }

    #[test]
    fn lint_catches_manifest_entry_hook_contradiction() {
        //  vector: agent.toml declares a `gate` hook on pre_cap/*, but
        // the entry file programmatically registers the SAME (event, match)
        // as `observe` — a silent contradiction the lint must catch.
        let tmp = tempdir().unwrap();
        let root = tmp.path().join("contradicts");
        scaffold(&root, "contradicts");

        let agent_toml = fs::read_to_string(root.join("agent.toml")).unwrap();
        let agent_toml = format!(
            "{agent_toml}\n[[hooks]]\nevent = \"pre_cap\"\nmatch = \"*\"\nmode = \"gate\"\nhandler = \"capabilities/handlers/guard.py:check\"\n"
        );
        fs::write(root.join("agent.toml"), agent_toml).unwrap();

        fs::create_dir_all(root.join("python")).unwrap();
        fs::write(
            root.join("python/contradicts_agent.py"),
            "from apxm import hook\n\n\
             @hook(on=\"pre_cap\", match=\"*\", mode=\"observe\")\n\
             def check(ctx):\n    return None\n",
        )
        .unwrap();

        let err = agent_lint(&root, None, true)
            .expect_err("manifest/entry hook contradiction must fail lint");
        assert!(
            err.to_string().contains("lint error"),
            "expected a lint error, got: {err}"
        );
    }

    #[test]
    fn lint_allows_matching_programmatic_hook() {
        // Same (event, match, mode) declared in both places is not a
        // contradiction — entry code may re-affirm what the manifest says.
        let tmp = tempdir().unwrap();
        let root = tmp.path().join("agrees");
        scaffold(&root, "agrees");

        let agent_toml = fs::read_to_string(root.join("agent.toml")).unwrap();
        let agent_toml = format!(
            "{agent_toml}\n[[hooks]]\nevent = \"pre_cap\"\nmatch = \"*\"\nmode = \"gate\"\nhandler = \"capabilities/handlers/guard.py:check\"\n"
        );
        fs::write(root.join("agent.toml"), agent_toml).unwrap();

        fs::create_dir_all(root.join("python")).unwrap();
        fs::write(
            root.join("python/agrees_agent.py"),
            "from apxm import hook\n\n\
             @hook(on=\"pre_cap\", match=\"*\", mode=\"gate\")\n\
             def check(ctx):\n    return None\n",
        )
        .unwrap();

        agent_lint(&root, None, true).expect("matching programmatic hook must not fail lint");
    }

    #[test]
    fn lint_allows_no_entry_with_runtime_loop() {
        // Pure-declarative looped agent: no entry file, [runtime.loop] declared.
        let tmp = tempdir().unwrap();
        let root = tmp.path().join("declarative");
        scaffold(&root, "declarative");

        agent_lint(&root, None, true).expect("entry-less declarative agent should lint clean");
    }

    #[test]
    fn lint_rejects_unrecognized_files() {
        let tmp = tempdir().unwrap();
        let root = tmp.path().join("stray");
        scaffold(&root, "stray");
        fs::write(root.join("not-a-real-file.txt"), "nope").unwrap();
        let err = agent_lint(&root, None, true).expect_err("unrecognized file must fail lint");
        assert!(err.to_string().contains("lint error"));
    }

    #[test]
    fn build_produces_verifiable_hash_chain() {
        let tmp = tempdir().unwrap();
        let root = tmp.path().join("hashed");
        scaffold(&root, "hashed");

        agent_build(&root, true).expect("build ok");

        let integrity: IntegrityToml = read_toml(&root.join("integrity.toml")).unwrap();
        assert_eq!(integrity.algorithm, "sha256");
        assert!(!integrity.chain.is_empty());
        assert_eq!(
            integrity.chain.last().unwrap().hash,
            integrity.hash,
            "hash must equal the final chain link's hash"
        );

        // Independently recompute every file's digest straight from the
        // finished agent on disk and confirm the whole chain
        // algebra: each link's hash == sha256(prev_hash || path || digest),
        // link 0's prev_hash is genesis, and hash is the last link.
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
        // env / real home — see `agent_install_to`'s doc comment).
        agent_install_to(&root, fake_home.path(), false, true).expect("install ok");

        let dest = fake_home.path().join("agents").join("installable");
        assert!(dest.join("agent.toml").is_file());
        assert!(dest.join("integrity.toml").is_file());
        assert!(dest.join("skills/installable-skill/skill.toml").is_file());

        // The real home directory must never be touched by this test.
        let real_home = dirs::home_dir().unwrap_or_default();
        assert!(!real_home.join(".apxm/agents/installable").exists());
    }

    #[test]
    fn install_refuses_existing_destination_without_force() {
        let tmp = tempdir().unwrap();
        let root = tmp.path().join("dupe");
        scaffold(&root, "dupe");
        let fake_home = tempdir().unwrap();

        agent_install_to(&root, fake_home.path(), false, true).expect("first install ok");
        let second = agent_install_to(&root, fake_home.path(), false, true);
        let err = second.expect_err("second install without --force must fail");
        assert!(err.to_string().contains("already exists"));
    }

    #[test]
    fn install_ignores_manual_libs_dir() {
        //  hard cutover: `~/.apxm/libs` is no longer a load or scan root.
        // Install must place the agent under agents/ and neither read nor touch
        // any stale copy sitting under the old libs convention.
        let tmp = tempdir().unwrap();
        let root = tmp.path().join("conflicted");
        scaffold(&root, "conflicted");
        let fake_home = tempdir().unwrap();

        let manual = fake_home.path().join("libs").join("conflicted");
        fs::create_dir_all(&manual).unwrap();
        fs::write(manual.join("marker.txt"), "stale libs copy, left untouched").unwrap();

        agent_install_to(&root, fake_home.path(), false, true).expect("install ok");

        assert!(manual.join("marker.txt").is_file());
        assert!(
            fake_home
                .path()
                .join("agents/conflicted/agent.toml")
                .is_file()
        );
    }

    // -----------------------------------------------------------------
    // skill compilation, hand-edit/hash-drift detection, org globals
    // -----------------------------------------------------------------

    /// Scaffold a `compiled = true` skill directory with the given frontend
    /// source, alongside the agent `scaffold()` already created.
    fn add_compiled_skill(root: &Path, id: &str, frontend: FrontendLanguage, source: &str) {
        let dir = root.join("skills").join(id);
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("skill.toml"),
            format!(
                "id = \"{id}\"\ncompiled = true\nfrontend = \"{frontend}\"\ncapabilities = []\n"
            ),
        )
        .unwrap();
        fs::write(dir.join("SKILL.md"), format!("# {id}\n")).unwrap();
        fs::write(dir.join("prompt.md"), "Prompt body.\n").unwrap();
        fs::write(dir.join(frontend.skill_source_filename()), source).unwrap();
    }

    const PYTHON_SKILL_SOURCE: &str = "from apxm import GraphRecorder, compile, emit_air_if_requested\n\n\n\
         @compile()\n\
         def my_skill(g: GraphRecorder):\n\
         \x20\x20\x20\x20ask = g.ask(name=\"respond\", prompt=\"Describe the weather today.\")\n\
         \x20\x20\x20\x20g.done(source=ask)\n\n\n\
         if __name__ == \"__main__\":\n\
         \x20\x20\x20\x20emit_air_if_requested(my_skill)\n";

    /// Same graph shape as [`PYTHON_SKILL_SOURCE`] (one `ask` -> `done`, same
    /// name/prompt), authored through `@apxm/frontend`'s `GraphBuilder`.
    /// TypeScript emits AIR directly from the generated op catalog.
    #[cfg(feature = "driver")]
    fn typescript_skill_source() -> String {
        String::from(
            "import { GraphBuilder } from \"@apxm/frontend\";\n\n\
             const g = new GraphBuilder(\"my_skill\");\n\
             const ask = g.ask({ name: \"respond\", prompt: \"Describe the weather today.\" });\n\
             g.done(ask);\n\
             console.log(g.toAir());\n",
        )
    }

    #[cfg(feature = "driver")]
    fn node_available() -> bool {
        std::process::Command::new("node")
            .arg("--version")
            .output()
            .is_ok_and(|o| o.status.success())
    }

    #[cfg(feature = "driver")]
    #[test]
    fn build_compiles_python_skill_to_air_and_apxmobj() {
        let tmp = tempdir().unwrap();
        let root = tmp.path().join("py-skill");
        scaffold(&root, "py-skill");
        add_compiled_skill(
            &root,
            "py-skill-skill",
            FrontendLanguage::Python,
            PYTHON_SKILL_SOURCE,
        );
        // Replace the scaffolded prompt-only skill.toml with the compiled one
        // (scaffold() already created skills/py-skill-skill/{SKILL.md,prompt.md}
        // with compiled = false; add_compiled_skill above overwrote skill.toml
        // and the source file in place).

        agent_build(&root, true).expect("build must compile the python skill");

        let skill_dir = root.join("skills/py-skill-skill");
        assert!(skill_dir.join("skill.air").is_file());
        assert!(skill_dir.join("skill.apxmobj").is_file());
        let air = fs::read_to_string(skill_dir.join("skill.air")).unwrap();
        assert!(air.contains("ais.ask \"Describe the weather today.\""));

        let skill: SkillToml = read_toml(&skill_dir.join("skill.toml")).unwrap();
        assert!(skill.source_hash.is_some());
        assert!(skill.air_hash.is_some());
        assert_eq!(
            skill.air_hash.as_deref(),
            Some(sha256_hex(air.as_bytes()).as_str())
        );
    }

    #[cfg(feature = "driver")]
    #[test]
    fn build_compiles_typescript_skill_to_air_and_apxmobj() {
        assert!(
            node_available(),
            "Node.js is required to validate the supported TypeScript frontend"
        );
        let tmp = tempdir().unwrap();
        let root = tmp.path().join("ts-skill");
        scaffold(&root, "ts-skill");
        let ts_source = typescript_skill_source();
        add_compiled_skill(
            &root,
            "ts-skill-skill",
            FrontendLanguage::TypeScript,
            &ts_source,
        );

        agent_build(&root, true).expect("build must compile the typescript skill");

        let skill_dir = root.join("skills/ts-skill-skill");
        assert!(skill_dir.join("skill.air").is_file());
        assert!(skill_dir.join("skill.apxmobj").is_file());
        let air = fs::read_to_string(skill_dir.join("skill.air")).unwrap();
        assert!(air.contains("ais.ask \"Describe the weather today.\""));
    }

    #[cfg(not(feature = "driver"))]
    #[test]
    fn build_of_compiled_skill_without_driver_feature_errors_clearly() {
        let tmp = tempdir().unwrap();
        let root = tmp.path().join("nodriver");
        scaffold(&root, "nodriver");
        add_compiled_skill(
            &root,
            "nodriver-skill",
            FrontendLanguage::Python,
            PYTHON_SKILL_SOURCE,
        );

        let err = agent_build(&root, true).expect_err("compiling requires the driver feature");
        assert!(err.to_string().contains("driver feature"));
    }

    #[test]
    fn lint_catches_hand_edited_compiled_artifact() {
        // A skill whose skill.toml already records (source_hash, air_hash)
        // from a prior build, but whose skill.air on disk no longer matches
        // air_hash even though the source is unchanged, must fail lint —
        // this is metadata-only (no compilation) so it runs with or without
        // the `driver` feature. See `detect_hand_edited_artifact`.
        let tmp = tempdir().unwrap();
        let root = tmp.path().join("handedit");
        scaffold(&root, "handedit");
        add_compiled_skill(
            &root,
            "handedit-skill",
            FrontendLanguage::Python,
            PYTHON_SKILL_SOURCE,
        );

        let dir = root.join("skills/handedit-skill");
        let original_air = "module {\n  func.func @handedit() -> !ais.token attributes {ais.entry} {\n    %r = ais.ask \"original\" : !ais.token\n    func.return %r : !ais.token\n  }\n}\n";
        fs::write(dir.join("skill.air"), original_air).unwrap();
        let source_hash = sha256_hex_file(&dir.join("skill.py")).unwrap();
        let air_hash = sha256_hex(original_air.as_bytes());
        fs::write(
            dir.join("skill.toml"),
            format!(
                "id = \"handedit-skill\"\ncompiled = true\nfrontend = \"{}\"\ncapabilities = []\n\
                 source_hash = \"{source_hash}\"\nair_hash = \"{air_hash}\"\n",
                FrontendLanguage::Python,
            ),
        )
        .unwrap();

        // Hand-edit skill.air without touching the source.
        fs::write(
            dir.join("skill.air"),
            "module {\n  hand edited garbage\n}\n",
        )
        .unwrap();

        let err = agent_lint(&root, None, true).expect_err("hand-edited artifact must fail lint");
        assert!(
            err.to_string().contains("lint error"),
            "expected a lint error, got: {err}"
        );
    }

    #[test]
    fn build_rejects_hand_edited_artifact_before_recompiling() {
        // Same drift as `lint_catches_hand_edited_compiled_artifact`, but
        // checked (and rejected) by `agent build` itself, before it would
        // otherwise recompile and silently clobber the hand edit. This does
        // not require the `driver` feature: the guard runs before
        // `compile_skill` is ever called.
        let tmp = tempdir().unwrap();
        let root = tmp.path().join("handedit-build");
        scaffold(&root, "handedit-build");
        add_compiled_skill(
            &root,
            "handedit-build-skill",
            FrontendLanguage::Python,
            PYTHON_SKILL_SOURCE,
        );

        let dir = root.join("skills/handedit-build-skill");
        let original_air = "module {\n  func.func @x() -> !ais.token attributes {ais.entry} {\n    %r = ais.ask \"original\" : !ais.token\n    func.return %r : !ais.token\n  }\n}\n";
        fs::write(dir.join("skill.air"), original_air).unwrap();
        let source_hash = sha256_hex_file(&dir.join("skill.py")).unwrap();
        let air_hash = sha256_hex(original_air.as_bytes());
        fs::write(
            dir.join("skill.toml"),
            format!(
                "id = \"handedit-build-skill\"\ncompiled = true\nfrontend = \"{}\"\ncapabilities = []\n\
                 source_hash = \"{source_hash}\"\nair_hash = \"{air_hash}\"\n",
                FrontendLanguage::Python,
            ),
        )
        .unwrap();

        // Hand-edit skill.air without touching the source: current air hash
        // no longer matches the recorded air_hash, even though source_hash
        // still matches current source — the unambiguous hand-edit signal.
        fs::write(
            dir.join("skill.air"),
            "module {\n  hand edited garbage\n}\n",
        )
        .unwrap();

        let err = agent_build(&root, true)
            .expect_err("build must refuse to recompile over a hand-edited artifact");
        assert!(
            err.to_string().contains("hand-editing"),
            "expected the hand-edit drift message, got: {err}"
        );
    }

    #[test]
    fn build_does_not_flag_first_build_with_no_recorded_hash() {
        // "no prior recorded hash = normal first build, not an error": a
        // freshly-scaffolded compiled skill with no skill.air yet and no
        // source_hash/air_hash recorded must not trip the drift guard.
        let tmp = tempdir().unwrap();
        let root = tmp.path().join("firstbuild");
        scaffold(&root, "firstbuild");
        add_compiled_skill(
            &root,
            "firstbuild-skill",
            FrontendLanguage::Python,
            PYTHON_SKILL_SOURCE,
        );

        let skill: SkillToml = read_toml(&root.join("skills/firstbuild-skill/skill.toml")).unwrap();
        assert!(
            detect_hand_edited_artifact(&root, &skill)
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn lint_allows_org_global_capability_without_local_join() {
        // A skill invoking an org-global capability must not be falsely
        // flagged as undeclared when the agent itself does not join it
        // locally.
        let tmp = tempdir().unwrap();
        let root = tmp.path().join("orgmember");
        scaffold(&root, "orgmember");
        fs::write(
            root.join("skills/orgmember-skill/skill.toml"),
            "id = \"orgmember-skill\"\ncompiled = false\ncapabilities = [\"org.shared_tool\"]\n",
        )
        .unwrap();

        // Without --org, the org-global capability is undeclared: lint fails.
        let err = agent_lint(&root, None, true).expect_err("undeclared capability must fail lint");
        assert!(err.to_string().contains("lint error"));

        // Scaffold a minimal org agent declaring that capability as a
        // joined org global.
        let org_root = tmp.path().join("org");
        fs::create_dir_all(org_root.join("capabilities")).unwrap();
        fs::write(
            org_root.join("capabilities/capabilities.toml"),
            "[[capability]]\nid = \"org.shared_tool\"\n",
        )
        .unwrap();
        fs::write(
            org_root.join("capabilities/permissions.toml"),
            "[[permission]]\ncapability = \"org.shared_tool\"\ndecision = \"allow\"\n",
        )
        .unwrap();

        agent_lint(&root, Some(org_root), true)
            .expect("org-global capability must satisfy lint with --org");
    }

    #[test]
    fn lint_still_catches_capability_absent_from_org_globals_too() {
        // Providing --org does not amnesty *every* undeclared capability —
        // only ones the org agent actually joins as globals.
        let tmp = tempdir().unwrap();
        let root = tmp.path().join("orgmember2");
        scaffold(&root, "orgmember2");
        fs::write(
            root.join("skills/orgmember2-skill/skill.toml"),
            "id = \"orgmember2-skill\"\ncompiled = false\ncapabilities = [\"nowhere.at.all\"]\n",
        )
        .unwrap();

        let org_root = tmp.path().join("org2");
        fs::create_dir_all(org_root.join("capabilities")).unwrap();
        fs::write(
            org_root.join("capabilities/capabilities.toml"),
            "[[capability]]\nid = \"org.other\"\n",
        )
        .unwrap();
        fs::write(
            org_root.join("capabilities/permissions.toml"),
            "[[permission]]\ncapability = \"org.other\"\ndecision = \"allow\"\n",
        )
        .unwrap();

        let err = agent_lint(&root, Some(org_root), true)
            .expect_err("a capability absent from both the agent and org globals must fail lint");
        assert!(err.to_string().contains("lint error"));
    }

    #[test]
    fn typescript_handler_toolchain_reports_dekk_prerequisites() {
        let tmp = tempdir().unwrap();
        let frontend = tmp.path().join("frontend");
        fs::create_dir_all(&frontend).unwrap();

        let missing_dependencies = validate_typescript_handler_toolchain(&frontend)
            .expect_err("missing dependencies must fail");
        assert!(
            missing_dependencies
                .to_string()
                .contains("dekk agents frontend setup")
        );

        fs::create_dir_all(frontend.join("node_modules/esbuild")).unwrap();
        let missing_build = validate_typescript_handler_toolchain(&frontend)
            .expect_err("missing compiler build must fail");
        assert!(
            missing_build
                .to_string()
                .contains("dekk agents frontend build")
        );

        fs::create_dir_all(frontend.join("dist")).unwrap();
        fs::write(frontend.join("dist/compile-handlers.js"), "export {};").unwrap();
        validate_typescript_handler_toolchain(&frontend)
            .expect("complete frontend toolchain must pass");
    }

    #[cfg(feature = "driver")]
    mod gao_e2e {
        include!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../../examples/agents/gao/tests/e2e_manifest.rs"
        ));
    }
}
