//! `apxm agent new|sync|lint|build|install|verify` — the toolchain for the
//! canonical agent folder format (`apxm.agent`).
//!
//! An agent package is two files: the authored `agent.toml` and the generated
//! `integrity.toml`. Everything a package can say about itself is said once —
//! the capability ids it can satisfy are the built-in allowlist plus the
//! `capabilities/<id>/handler.{py,ts}` handlers it ships, and the only authored
//! permission surface is `agent.toml [permissions]`. There is no capability
//! inventory, no permission inventory, and no separate hierarchy file to drift
//! against each other.
//!
//! The recognized folder contract is not restated here: it is derived from
//! `contracts/schemas/apxm.agent.json`'s `files` patternProperties, so the
//! published schema and the predicate `agent lint` and `agent verify` enforce
//! cannot disagree.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::{self, Write};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use anyhow::{Context, Result, anyhow, bail};
use apxm_ais::permissions::{LayerDecisions, PermissionDecision, PermissionResolution};
use apxm_core::types::{HandlerKind, HandlerLanguage, HandlerManifest};
use regex::Regex;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::implementations::{Status, print_section_header, print_status_line};

// ---------------------------------------------------------------------
// On-disk manifest shapes (apxm.agent projections)
// ---------------------------------------------------------------------

const AGENT_SCHEMA: &str = "apxm.agent";

/// The published `apxm.agent` contract, embedded from its checked-in bytes.
///
/// The folder contract has exactly one statement, and this is it. Deriving the
/// recognized-path predicate from these bytes is what makes the collapse
/// enforceable: a package still carrying `capabilities/capabilities.toml` is
/// rejected because the schema does not name that path, not because a
/// hand-written `match` arm happened to be deleted alongside it.
pub(crate) const AGENT_SCHEMA_JSON: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../../contracts/schemas/apxm.agent.json"
));

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

/// Projection of `agent.toml` (`apxm.agent#/properties/agent`).
///
/// Scalars come first and table-valued fields last so a round-trip through
/// `toml::to_string_pretty` — which `agent new` performs when it rewrites a
/// scaffolded identity — emits valid TOML. `deny_unknown_fields` is what makes
/// a retired key (`capabilities`, `allowed_agent_skills`, `hooks`, `prompts`) a
/// parse error rather than a silently dropped table.
///
/// `prompts` is retired rather than absent by accident. It was parsed here,
/// scaffolded by `agent new`, and hashed into the integrity chain, but nothing
/// ever loaded a declared path or even checked that it existed — a manifest key
/// promising behaviour no consumer delivered. The same handler-first rule that
/// keeps an unimplemented capability id out of the builtin allowlist keeps it
/// out until a loader lands. `prompts/*.md` stays a recognized package file;
/// what is gone is the table claiming those files are loaded.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentToml {
    pub id: String,
    pub version: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub schema_version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub domain: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub license: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub compile: Option<CompileToml>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<toml::Value>,
    /// This agent's parent and the children it may spawn or delegate to.
    /// Absorbed from the retired `hierarchy.toml`: one manifest, one place.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hierarchy: Option<HierarchyToml>,
    /// The package layer of the permission resolution stack: what this package
    /// decides about a capability it can actually supply. It may only tighten
    /// the unqualified request a capability reference is, and it may not decide
    /// for a capability nothing grants.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub permissions: BTreeMap<String, PermissionDecision>,
}

/// Projection of `agent.toml`'s `[hierarchy]` table.
///
/// Shared with `org.rs`, whose `members.toml` entries carry a snapshot of this
/// exact shape so org lint can check a member against `topology.toml` without
/// resolving the installed agent.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HierarchyToml {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub permitted_children: Vec<String>,
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
}

impl fmt::Display for FrontendLanguage {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Python => "python",
            Self::TypeScript => "typescript",
        })
    }
}

const TYPESCRIPT_AGENT_PACKAGING_PACKAGE: &str = "APXM_TYPESCRIPT_AGENT_PACKAGING_PACKAGE";
const PYTHON_AGENT_PACKAGING_PACKAGE: &str = "APXM_PYTHON_AGENT_PACKAGING_PACKAGE";

/// Everything one handler language differs by: the extension its shipped source
/// has, which interpreter runs its private packaging entries, where that package
/// root is named, and the two entry file names.
///
/// This is the whole language-specific surface. Discovery, bundling, and worker
/// selection all read it, so adding a language is one arm rather than a search
/// for every place the TypeScript pair was spelled.
struct AgentPackaging {
    extension: &'static str,
    interpreter: &'static str,
    package_variable: &'static str,
    compiler: &'static str,
    worker: &'static str,
}

const fn agent_packaging(language: HandlerLanguage) -> AgentPackaging {
    match language {
        HandlerLanguage::Python => AgentPackaging {
            extension: "py",
            interpreter: "python3",
            package_variable: PYTHON_AGENT_PACKAGING_PACKAGE,
            compiler: "compile_handlers.py",
            worker: "tool_worker.py",
        },
        HandlerLanguage::TypeScript => AgentPackaging {
            extension: "ts",
            interpreter: "node",
            package_variable: TYPESCRIPT_AGENT_PACKAGING_PACKAGE,
            compiler: "compile-handlers.mjs",
            worker: "tool-worker.mjs",
        },
    }
}

/// Every language a package may ship a handler in.
const HANDLER_LANGUAGES: [HandlerLanguage; 2] =
    [HandlerLanguage::Python, HandlerLanguage::TypeScript];

fn installed_agent_packaging_entry(language: HandlerLanguage, relative: &str) -> Result<PathBuf> {
    let variable = agent_packaging(language).package_variable;
    let package = std::env::var_os(variable).ok_or_else(|| {
        anyhow!("{variable} must point to the installed {language:?} agent-packaging package")
    })?;
    let entry = PathBuf::from(package).join(relative);
    if !entry.is_file() {
        bail!(
            "installed {language:?} agent-packaging entry is missing: {}",
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
        super::AgentAction::Verify { path } => agent_verify(&path, json_output),
    }
}

/// Hold a checked-in package against its own generated integrity chain.
///
/// `agent lint` reads the package as authored and `agent build` writes the
/// chain; neither notices a package edited after its last build. This does, so
/// a package whose bytes moved without a rebuild fails a gate instead of
/// shipping a chain that describes something else.
fn agent_verify(path: &Path, json_output: bool) -> Result<()> {
    verify_agent_integrity(path)?;
    let pkg = load_agent(path)?;
    let integrity: IntegrityToml = read_toml(&path.join("integrity.toml"))?;
    if json_output {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "id": pkg.agent.id,
                "path": path.display().to_string(),
                "files": integrity.chain.len(),
                "hash": integrity.hash,
                "status": "verified",
            }))?
        );
    } else {
        print_section_header("Agent Verify");
        print_status_line(
            &pkg.agent.id,
            Status::Ok,
            &format!(
                "{} files verified, hash={}",
                integrity.chain.len(),
                integrity.hash
            ),
        );
    }
    Ok(())
}

fn default_agent_root(id: &str) -> PathBuf {
    PathBuf::from("agents").join(id)
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
    write_new_file(
        &root.join("agent.toml"),
        &format!(
            "id = \"{id}\"\n\
             version = \"0.1.0\"\n\
             schema_version = \"{AGENT_SCHEMA}\"\n\
             display_name = \"{display_name}\"\n\
             domain = \"{id}\"\n\
             kind = \"agent\"\n\
             license = \"MIT\"\n\n\
             [compile]\n\
             frontend = \"typescript\"\n\
             entry = \"src/main.ts\"\n\n\
             [source]\n\
             type = \"local\"\n\n\
             # Optional: this agent's parent and the children it may spawn or\n\
             # delegate to.\n\
             [hierarchy]\n\
             permitted_children = []\n\n\
             # Optional: tighten a capability this package can supply. A key\n\
             # must name a built-in id or a capabilities/<id>/handler this\n\
             # package ships.\n\
             [permissions]\n\
             write = \"ask\"\n"
        ),
    )?;

    write_new_file(
        &root.join("package.json"),
        &format!("{{\n  \"name\": \"{id}\",\n  \"private\": true,\n  \"type\": \"module\"\n}}\n"),
    )?;
    write_new_file(
        &root.join("tsconfig.json"),
        "{\n  \"compilerOptions\": {\n    \"target\": \"ES2022\",\n    \"module\": \"ESNext\",\n    \"moduleResolution\": \"bundler\",\n    \"strict\": true\n  }\n}\n",
    )?;

    // A persona is instructions and supporting context, which is what a skill
    // is. It is scaffolded as one so the file a package writes is a file a
    // program can load — `Skill("persona", entry="skills/persona/SKILL.md")`
    // and `await persona.load()`. The retired `[prompts]` table pointed at a
    // file nothing read; this points at the loader that now exists.
    write_new_file(
        &root.join("skills/persona/SKILL.md"),
        &format!(
            "---\n\
             name: persona\n\
             description: How the {display_name} agent presents itself and what it is for.\n\
             ---\n\n\
             # {display_name}\n\n\
             Describe this agent's persona here.\n"
        ),
    )?;
    write_new_file(
        &root.join("src/main.ts"),
        "// Define this package's explicit APXM entry with @apxm/frontend.\n",
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

fn load_tools_manifest(root: &Path) -> Result<HandlerManifest> {
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

/// Project each shipped Tool's resolved permission decision onto its handler
/// manifest entry.
///
/// This is the handler plane's whole authority story now that the per-capability
/// manifests are gone. `requires_approval` is not a second opinion about a
/// capability: it is the one decision `resolve_permission_layers` produced for
/// that capability id, carried to the only consumer that reads a manifest.
///
/// The decision is about a capability id, so it is projected onto every
/// descriptor regardless of the language that emitted it: a Python handler and a
/// TypeScript handler for the same id would be the same authority question.
///
/// A handler the package ships for a capability the resolution denies is a hard
/// error, not a filtered entry: emitting an executable handler for refused
/// authority is exactly the state the manifest exists to make impossible.
fn apply_resolved_permissions_to_tools_manifest(
    manifest: &mut HandlerManifest,
    resolved: &BTreeMap<String, PermissionDecision>,
) -> Result<()> {
    for entry in manifest
        .handlers
        .iter_mut()
        .filter(|entry| entry.kind == HandlerKind::Tool)
    {
        // The decision vocabulary is closed at decode, so there is no
        // unsupported-string arm left to write: only the three decisions and an
        // absent one can reach this match.
        let requires_approval = match resolved.get(entry.name.as_str()) {
            Some(PermissionDecision::Allow { .. }) => false,
            Some(PermissionDecision::Ask { .. }) => true,
            Some(PermissionDecision::Deny { .. }) => bail!(
                "capability '{}' is denied and cannot be emitted as an executable handler",
                entry.name
            ),
            None => bail!(
                "handler '{}' has no matching capability; a handler must live at \
                 capabilities/{}/handler.{} so the package can supply the capability it names",
                entry.name,
                entry.name,
                agent_packaging(entry.language).extension,
            ),
        };
        entry.requires_approval = Some(requires_approval);
    }
    Ok(())
}

fn write_tools_manifest(root: &Path, manifest: &HandlerManifest) -> Result<()> {
    manifest
        .validate()
        .context("Invalid joined handler manifest")?;
    let path = root.join("capabilities/handlers/tools.json");
    if manifest.handlers.is_empty() {
        if path.is_file() {
            fs::remove_file(&path)
                .with_context(|| format!("Failed to remove stale {}", path.display()))?;
        }
        return Ok(());
    }
    let data =
        serde_json::to_vec_pretty(manifest).context("Failed to serialize handler manifest")?;
    fs::write(&path, [data.as_slice(), b"\n"].concat())
        .with_context(|| format!("Failed to write {}", path.display()))
}

/// Every Capability id an Agent Program compiled from this package may name.
///
/// The union of the runtime's built-in allowlist and the ids the package itself
/// ships a `capabilities/<id>/handler.{py,ts}` for — the two namespaces a
/// Capability reference can be satisfied from, now that no file declares an
/// inventory. The handler's existence *is* the declaration: a package that ships
/// a handler can supply the capability, and one that does not cannot.
pub(crate) fn granted_capability_ids(root: &Path) -> Result<BTreeSet<String>> {
    Ok(apxm_ais::capabilities::BUILTINS
        .iter()
        .map(|id| (*id).to_string())
        .chain(shipped_capability_handlers(root)?.into_keys())
        .collect())
}

/// Resolve the package-handler implementations one package supplies.
///
/// This is the join that makes [`granted_capability_ids`] an honest claim.
/// The ⊆ gate calls a shipped `capabilities/<id>/handler.ts` grantable, so
/// this refuses unless the built manifest carries an executable descriptor for
/// exactly those ids: a handler source with no manifest entry would be a grant
/// nothing can dispatch, and a manifest entry with no handler source would be
/// executable code the package never declared.
///
/// A package that ships no handler supplies no implementation, which is `None`
/// rather than an empty binding: the private worker is only resolved when there
/// is something for it to evaluate.
///
/// # Errors
///
/// Returns an error when the package fails integrity verification, when its
/// manifest is absent or non-conforming, when the manifest and the shipped
/// handler sources disagree, or when a private worker the manifest needs is not
/// installed.
pub(crate) fn admitted_package_handlers(
    root: &Path,
) -> Result<Option<super::canonical_execute::AdmittedPackageHandlers>> {
    verify_agent_integrity(root)?;
    let manifest = load_tools_manifest(root)?;
    let described: BTreeMap<String, HandlerLanguage> = manifest
        .handlers
        .iter()
        .map(|entry| (entry.name.clone(), entry.language))
        .collect();
    let shipped = shipped_capability_handlers(root)?;
    if described != shipped {
        let names = |handlers: &BTreeMap<String, HandlerLanguage>| {
            handlers
                .iter()
                .map(|(name, language)| format!("{name} ({language:?})"))
                .collect::<Vec<_>>()
                .join(", ")
        };
        bail!(
            "agent package '{}' would grant [{}] but ships executable handlers for [{}]; run \
             'apxm agent build {}' so the ids the grant set claims are exactly the ids the \
             runtime can dispatch",
            root.display(),
            names(&shipped),
            names(&described),
            root.display()
        );
    }
    if manifest.handlers.is_empty() {
        return Ok(None);
    }
    // Only the languages this manifest actually uses are resolved, so a package
    // shipping one language never requires the other language's worker.
    let mut workers = BTreeMap::new();
    for language in described.into_values() {
        if let std::collections::btree_map::Entry::Vacant(slot) = workers.entry(language) {
            let packaging = agent_packaging(language);
            slot.insert(super::canonical_execute::PackageHandlerWorkerCommand {
                interpreter: packaging.interpreter.to_string(),
                entry: installed_agent_packaging_entry(language, packaging.worker)?,
            });
        }
    }
    Ok(Some(super::canonical_execute::AdmittedPackageHandlers {
        workers,
        manifest,
    }))
}

/// The capability ids this package ships a handler for, and the language of each.
///
/// Discovery is the folder contract: `capabilities/<id>/handler.py` and
/// `capabilities/<id>/handler.ts` are the same declaration in the two frontends,
/// so both are found here and the extension is the only thing that differs.
///
/// One id may be supplied once. A directory holding both handler sources would
/// be two implementations of one capability with nothing to choose between them,
/// so it is refused rather than resolved by an order this function picked.
fn shipped_capability_handlers(root: &Path) -> Result<BTreeMap<String, HandlerLanguage>> {
    let caps_dir = root.join("capabilities");
    if !caps_dir.is_dir() {
        return Ok(BTreeMap::new());
    }
    let mut shipped = BTreeMap::new();
    for entry in
        fs::read_dir(&caps_dir).with_context(|| format!("Failed to read {}", caps_dir.display()))?
    {
        let path = entry?.path();
        if !path.is_dir() {
            continue;
        }
        let id = path
            .file_name()
            .expect("a directory entry has a file name")
            .to_string_lossy()
            .into_owned();
        for language in HANDLER_LANGUAGES {
            if !path
                .join(format!("handler.{}", agent_packaging(language).extension))
                .is_file()
            {
                continue;
            }
            if let Some(existing) = shipped.insert(id.clone(), language) {
                bail!(
                    "capability '{id}' ships both a {existing:?} and a {language:?} handler; one \
                     capability is supplied by one implementation"
                );
            }
        }
    }
    Ok(shipped)
}

/// Resolve the package's permission layer stack into one decision per capability
/// the package can supply.
///
/// **Code layer.** `authored` is what the compiled program's own source
/// requested, keyed by `capability_ref` — the
/// `CapabilityRequirement.requested_permission` an author wrote, carried this
/// far by `AirModule::capability_permission_requests`. A grantable id the
/// program stated no permission for gets a bare `allow`: authoring a Capability
/// reference is the program asking to invoke it, unqualified, and that is the
/// widest a request can be, which is the correct floor for a tighten-only stack.
///
/// The keyspace is the package's whole grantable surface rather than only what
/// the program invokes, because `agent.toml` is package policy: it may state a
/// decision for a capability this package can supply before any program invokes
/// it. What it may not do is decide for a capability *nothing* grants — there is
/// no request there for it to narrow.
///
/// **Callers supply what they know.** `agent lint` and `agent sync` read a
/// package as authored and compile no program, so they pass no authored
/// requests and every grantable id sits at the `allow` floor; `agent.toml` can
/// only narrow, never widen, and the widening arm is unreachable from there by
/// construction. `compile-service-canonical`, which has just lowered the
/// program, passes the AIR's authored requests, and that is where a package
/// trying to hand back authority the program itself declined is refused.
///
/// **Package layer.** `agent.toml [permissions]` is what this package decides on
/// top of that. It may narrow `allow` to `ask` or `deny`, and it may not widen
/// an authored request.
fn resolve_permission_layers(
    agent: &AgentToml,
    grantable: &BTreeSet<String>,
    authored: &LayerDecisions,
) -> Result<BTreeMap<String, PermissionDecision>, String> {
    let requested: LayerDecisions = grantable
        .iter()
        .map(|id| {
            (
                id.clone(),
                authored
                    .get(id)
                    .cloned()
                    .unwrap_or_else(PermissionDecision::allow),
            )
        })
        .collect();
    let resolution = PermissionResolution::resolve_code_over_package(
        requested,
        agent.permissions.clone().into_iter().collect(),
    )
    .map_err(|error| format!("[permissions]: {error}"))?;
    Ok(resolution
        .iter()
        .map(|(capability_ref, resolved)| (capability_ref.to_string(), resolved.decision.clone()))
        .collect())
}

/// Resolve one on-disk package's permission layer stack against what its
/// compiled program requested.
///
/// The seam `compile-service-canonical` reaches through: it holds the lowered
/// AIR's authored requests and the package root, and this joins them to the
/// package's grantable surface and its `agent.toml [permissions]` without
/// making the compile service a second owner of the stacking rule.
///
/// # Errors
///
/// Returns an error when the package cannot be read, or when `agent.toml`
/// widens an authored request or decides for a capability nothing grants.
pub(crate) fn resolve_package_permission_layers(
    root: &Path,
    authored: &LayerDecisions,
) -> Result<BTreeMap<String, PermissionDecision>> {
    let pkg = load_agent(root)?;
    let grantable = granted_capability_ids(root)?;
    resolve_permission_layers(&pkg.agent, &grantable, authored)
        .map_err(|error| anyhow!("{} {error}", root.join("agent.toml").display()))
}

pub(crate) fn agent_sync(root: &Path, json_output: bool) -> Result<()> {
    let pkg = load_agent(root)?;
    // Sync compiles no program, so it states no authored request and every
    // grantable id sits at the code layer's `allow` floor.
    let resolved = resolve_package_permission_layers(root, &LayerDecisions::new())?;

    compile_agent_handlers(root)?;
    let mut tools_manifest = load_tools_manifest(root)?;
    apply_resolved_permissions_to_tools_manifest(&mut tools_manifest, &resolved)?;
    write_tools_manifest(root, &tools_manifest)?;

    let handler_count = tools_manifest.handlers.len();
    if json_output {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "path": root.display().to_string(),
                "handlers": handler_count,
                "status": "synced",
            }))?
        );
    } else {
        print_section_header("Agent Sync");
        print_status_line(
            &pkg.agent.id,
            Status::Ok,
            &format!("{handler_count} handlers regenerated"),
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
}

fn load_agent(root: &Path) -> Result<LoadedAgent> {
    if !root.is_dir() {
        bail!("'{}' is not a directory", root.display());
    }
    let agent_path = root.join("agent.toml");
    if !agent_path.is_file() {
        bail!("missing required file: {}", agent_path.display());
    }
    Ok(LoadedAgent {
        root: root.to_path_buf(),
        agent: read_toml(&agent_path)?,
    })
}

// ---------------------------------------------------------------------
// agent lint
// ---------------------------------------------------------------------

/// The recognized-path patterns published by
/// `apxm.agent#/$defs/PackageFiles/patternProperties`, compiled once.
///
/// The schema's property names are ECMA-262 regexes and are used here as Rust
/// regexes; both are unanchored searches over the whole path, and every
/// published pattern anchors itself with `^`/`$`, so the two agree by
/// construction. Deriving the predicate rather than restating it is what keeps
/// a retired path — `capabilities/capabilities.toml`, `hierarchy.toml` — from
/// being tolerated by a `match` arm nobody remembered to delete.
fn recognized_relpath_patterns() -> &'static [Regex] {
    static PATTERNS: OnceLock<Vec<Regex>> = OnceLock::new();
    PATTERNS.get_or_init(|| {
        let schema: serde_json::Value = serde_json::from_str(AGENT_SCHEMA_JSON)
            .expect("the embedded apxm.agent contract is valid JSON");
        schema["$defs"]["PackageFiles"]["patternProperties"]
            .as_object()
            .expect("apxm.agent publishes PackageFiles.patternProperties")
            .keys()
            .map(|pattern| {
                Regex::new(pattern).unwrap_or_else(|error| {
                    panic!("apxm.agent publishes an uncompilable file pattern {pattern:?}: {error}")
                })
            })
            .collect()
    })
}

/// Is `rel` a package-relative path the published folder contract recognizes?
/// Anything else fails validation ("no agent-private layout").
fn recognized_relpath(rel: &str) -> bool {
    recognized_relpath_patterns()
        .iter()
        .any(|pattern| pattern.is_match(rel))
}

/// Walk the agent root and return every recognized file as a
/// (relative-path, absolute-path) pair, sorted by relative path.
fn walk_recognized_files(root: &Path) -> Result<Vec<(String, PathBuf)>> {
    let mut out = Vec::new();
    for entry in walkdir::WalkDir::new(root) {
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
/// schema's folder contract (excluding common noise and local build sidecars).
fn find_unrecognized_files(root: &Path) -> Result<Vec<String>> {
    let mut out = Vec::new();
    for entry in walkdir::WalkDir::new(root).into_iter().filter_entry(|e| {
        let name = e.file_name().to_string_lossy();
        !matches!(
            name.as_ref(),
            ".git" | "__pycache__" | ".pytest_cache" | "node_modules" | "dist"
        )
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

/// Resolve the package's permission layer stack and report every refusal.
///
/// See [`resolve_permission_layers`] for the two layers. `org_globals` extends
/// the grantable surface with the org's own joined global capability set, so a
/// member package may state a decision for a capability its org supplies.
fn check_permission_resolution(pkg: &LoadedAgent, org_globals: &BTreeSet<String>) -> Vec<String> {
    let mut grantable = match granted_capability_ids(&pkg.root) {
        Ok(grantable) => grantable,
        Err(error) => return vec![error.to_string()],
    };
    grantable.extend(org_globals.iter().cloned());
    match resolve_permission_layers(&pkg.agent, &grantable, &LayerDecisions::new()) {
        Ok(_) => Vec::new(),
        Err(error) => vec![format!("agent.toml {error}")],
    }
}

/// Structural + required-field checks for `apxm.agent`: id/version, schema
/// version, the compiled source declaration, and the absorbed hierarchy.
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
        Some(AGENT_SCHEMA) => {}
        Some(other) => errors.push(format!(
            "agent.toml: schema_version '{other}' must be '{AGENT_SCHEMA}'"
        )),
        None => errors.push(format!(
            "agent.toml: schema_version is required and must be '{AGENT_SCHEMA}'"
        )),
    }
    match declared_compile_source(&pkg.agent) {
        Ok(None) => errors.push(
            "agent.toml: an explicit [compile].entry is required; runtime loop declarations are unsupported"
                .to_string(),
        ),
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
    if let Some(hierarchy) = &pkg.agent.hierarchy
        && let Some(parent) = &hierarchy.parent
        && parent.trim().is_empty()
    {
        errors.push("agent.toml: [hierarchy].parent must not be empty when present".to_string());
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

fn semver_like(version: &str) -> bool {
    // SemVer core: MAJOR.MINOR.PATCH, optionally with -prerelease/+build.
    let core = version.split(['-', '+']).next().unwrap_or(version);
    let parts: Vec<&str> = core.split('.').collect();
    parts.len() == 3
        && parts
            .iter()
            .all(|p| !p.is_empty() && p.chars().all(|c| c.is_ascii_digit()))
}

pub(crate) fn agent_lint(path: &Path, org: Option<PathBuf>, json_output: bool) -> Result<()> {
    let pkg = load_agent(path)?;
    let org_globals = match &org {
        Some(org_root) => super::org::load_org_global_capabilities(org_root)
            .with_context(|| format!("Failed to load org globals from {}", org_root.display()))?,
        None => BTreeSet::new(),
    };

    let mut errors = check_schema_shape(&pkg);
    errors.extend(check_permission_resolution(&pkg, &org_globals));
    for unrecognized in find_unrecognized_files(path)? {
        errors.push(format!(
            "unrecognized file '{unrecognized}' is not part of the {AGENT_SCHEMA} folder contract"
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
    let mut encoded = String::with_capacity(digest.len() * 2);
    for byte in digest {
        write!(&mut encoded, "{byte:02x}").expect("writing to a string cannot fail");
    }
    encoded
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
        .map_or_else(|| genesis.to_string(), |link| link.hash.clone());
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
pub(crate) fn verify_agent_integrity(root: &Path) -> Result<()> {
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

#[cfg(all(test, feature = "driver"))]
pub(super) fn seal_agent_integrity_for_test(root: &Path) -> Result<()> {
    let integrity = compute_integrity(&digest_recognized_files(root)?);
    write_integrity_toml(&root.join("integrity.toml"), &integrity)
}

/// Every handler source of one language the package ships.
///
/// Discovery is the folder contract itself: a `capabilities/<id>/handler.<ext>`
/// is the package's declaration that it supplies capability `<id>`, and a
/// `capabilities/handlers/*.<ext>` is a shared handler module. Neither needs a
/// second file restating what the first one already says by existing.
fn collect_handler_sources(root: &Path, language: HandlerLanguage) -> Result<Vec<PathBuf>> {
    let extension = agent_packaging(language).extension;
    let mut sources = BTreeSet::new();
    let handlers_dir = root.join("capabilities/handlers");
    if handlers_dir.is_dir() {
        for entry in fs::read_dir(&handlers_dir)
            .with_context(|| format!("Failed to read {}", handlers_dir.display()))?
        {
            let path = entry?.path();
            if path.extension().and_then(|ext| ext.to_str()) == Some(extension) {
                sources.insert(path);
            }
        }
    }

    let caps_dir = root.join("capabilities");
    if caps_dir.is_dir() {
        for entry in fs::read_dir(&caps_dir)
            .with_context(|| format!("Failed to read {}", caps_dir.display()))?
        {
            let handler = entry?.path().join(format!("handler.{extension}"));
            if handler.is_file() {
                sources.insert(handler);
            }
        }
    }

    Ok(sources.into_iter().collect())
}

/// Run one language's bundler over its sources and return what it emitted.
///
/// Each bundler writes a whole manifest to a path this chooses, rather than to
/// the package's `tools.json`, so a package shipping both languages joins two
/// manifests here instead of having one bundler overwrite the other's output.
fn bundle_handlers(
    root: &Path,
    language: HandlerLanguage,
    sources: &[PathBuf],
) -> Result<HandlerManifest> {
    let packaging = agent_packaging(language);
    let compiler = installed_agent_packaging_entry(language, packaging.compiler)?;
    let out = tempfile::NamedTempFile::new()
        .context("Failed to materialize a handler-manifest build path")?
        .into_temp_path();

    let mut command = std::process::Command::new(packaging.interpreter);
    match language {
        // The Python bundler is a script with the same argv contract, so it is
        // invoked directly.
        HandlerLanguage::Python => {
            command.arg(&compiler);
        }
        // The TypeScript bundler exports a function rather than running one, so
        // the module is imported and called.
        HandlerLanguage::TypeScript => {
            command
                .arg("--input-type=module")
                .arg("--eval")
                .arg(
                    "import { pathToFileURL } from 'node:url'; import(pathToFileURL(process.argv[1]).href).then(async ({ compileHandlers }) => { const fs = await import('node:fs/promises'); const [out, root, ...sources] = process.argv.slice(2); const manifest = await compileHandlers(sources, { rootDir: root }); await fs.writeFile(out, `${JSON.stringify(manifest, null, 2)}\\n`); })",
                )
                .arg(&compiler);
        }
    }
    let status = command
        .arg(&out)
        .arg(root)
        .args(sources)
        .status()
        .with_context(|| {
            format!(
                "Failed to run installed {language:?} handler compiler {} (set {} to its package root)",
                compiler.display(),
                packaging.package_variable,
            )
        })?;
    if !status.success() {
        bail!("installed {language:?} handler compiler failed for agent handlers");
    }

    let manifest = HandlerManifest::from_json_slice(
        &fs::read(&out).context("Failed to read the compiled handler manifest")?,
    )
    .with_context(|| {
        format!("the {language:?} handler compiler emitted a non-conforming manifest")
    })?;
    Ok(manifest)
}

fn compile_agent_handlers(root: &Path) -> Result<()> {
    let root = root.canonicalize().with_context(|| {
        format!(
            "Failed to resolve absolute path for agent root {}",
            root.display()
        )
    })?;
    let mut handlers = Vec::new();
    for language in HANDLER_LANGUAGES {
        let sources = collect_handler_sources(&root, language)?;
        if sources.is_empty() {
            continue;
        }
        handlers.extend(bundle_handlers(&root, language, &sources)?.handlers);
    }

    let out = root.join("capabilities/handlers/tools.json");
    if handlers.is_empty() {
        if out.is_file() {
            fs::remove_file(&out)
                .with_context(|| format!("Failed to remove stale {}", out.display()))?;
        }
        return Ok(());
    }
    handlers.sort_by(|left, right| left.name.cmp(&right.name));
    fs::create_dir_all(out.parent().expect("tools.json has parent"))
        .with_context(|| format!("Failed to create {}", out.parent().unwrap().display()))?;
    write_tools_manifest(&root, &HandlerManifest::new(handlers))
}

pub(crate) fn agent_build(path: &Path, json_output: bool) -> Result<()> {
    agent_sync(path, false)?;
    let pkg = load_agent(path)?;
    // Hash the post-sync package metadata and instruction resources. Program
    // content is compiled from the package-level [compile] entry.
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
            }))?
        );
    } else {
        print_section_header("Agent Build");
        print_status_line(
            &pkg.agent.id,
            Status::Ok,
            &format!("{} files hashed, hash={}", files.len(), integrity.hash),
        );
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
    use apxm_core::types::{HandlerDescriptor, HandlerSource};
    use tempfile::tempdir;

    fn scaffold(dir: &Path, id: &str) {
        agent_new(id, Some(dir.to_path_buf()), None, "looped-agent", true).expect("scaffold ok");
    }

    #[test]
    fn new_scaffolds_the_two_manifest_package() {
        let tmp = tempdir().unwrap();
        let root = tmp.path().join("demo");
        scaffold(&root, "demo");

        for rel in [
            "agent.toml",
            "package.json",
            "tsconfig.json",
            "skills/persona/SKILL.md",
            "examples/basic.md",
            "tests/README.md",
        ] {
            assert!(root.join(rel).is_file(), "missing scaffolded file: {rel}");
        }
        // The persona is scaffolded where a program can load it from, not as a
        // `prompts/` file with no reader. `prompts/` stays a recognized path
        // for packages that already carry one; nothing new is written there.
        assert!(!root.join("prompts/persona.md").exists());
        // The collapse is the point: a fresh package carries no capability
        // inventory, no permission inventory, and no separate hierarchy file.
        for rel in [
            "hierarchy.toml",
            "capabilities/capabilities.toml",
            "capabilities/permissions.toml",
            "capabilities/read/capability.toml",
            "capabilities/write/permission.toml",
        ] {
            assert!(!root.join(rel).exists(), "scaffold resurrected {rel}");
        }

        let agent: AgentToml = read_toml(&root.join("agent.toml")).unwrap();
        assert_eq!(agent.id, "demo");
        assert_eq!(agent.version, "0.1.0");
        assert_eq!(agent.schema_version.as_deref(), Some(AGENT_SCHEMA));
        assert_eq!(
            agent
                .compile
                .as_ref()
                .and_then(|compile| compile.entry.as_deref()),
            Some("src/main.ts")
        );
        assert_eq!(agent.kind.as_deref(), Some("agent"));
        assert!(agent.hierarchy.is_some());
        assert!(matches!(
            agent.permissions.get("write"),
            Some(PermissionDecision::Ask { .. })
        ));

        agent_lint(&root, None, true).expect("scaffolded agent should lint clean");
        agent_build(&root, true).expect("scaffolded agent should build");
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
            text.replace("schema_version = \"apxm.agent\"\n", ""),
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

    /// The keys the collapse retired are parse errors, not tolerated extras.
    /// A package still carrying one is refused at decode, which is what makes
    /// "no space for old things" enforceable rather than aspirational.
    #[test]
    fn agent_toml_rejects_every_retired_key() {
        for retired in [
            "capabilities = [\"read\"]",
            "allowed_agent_skills = []",
            "chat = { enabled = true }",
            "[[hooks]]\nevent = \"pre_turn\"\nmode = \"observe\"\nhandler = \"h\"",
        ] {
            let document = format!("id = \"demo\"\nversion = \"0.1.0\"\n{retired}\n");
            let error = toml::from_str::<AgentToml>(&document)
                .expect_err("a retired key must not decode: {retired}");
            assert!(
                error.to_string().contains("unknown field"),
                "expected an unknown-field refusal for {retired:?}, got: {error}"
            );
        }
    }

    #[test]
    fn new_refuses_nonempty_destination() {
        let tmp = tempdir().unwrap();
        let root = tmp.path().join("demo");
        scaffold(&root, "demo");
        let err = agent_new("demo", Some(root.clone()), None, "looped-agent", true).unwrap_err();
        assert!(err.to_string().contains("already exists"));
    }

    /// The package's capability surface is the built-in allowlist plus the
    /// handlers it ships. Nothing declares an inventory, so nothing can drift
    /// from one.
    #[test]
    fn the_grantable_surface_is_builtins_plus_shipped_handlers() {
        let tmp = tempdir().unwrap();
        let root = tmp.path().join("granting");
        scaffold(&root, "granting");

        let builtins_only = granted_capability_ids(&root).unwrap();
        assert!(builtins_only.contains("read"));
        assert!(builtins_only.contains("write"));
        assert!(!builtins_only.contains("propose_edit"));

        fs::create_dir_all(root.join("capabilities/propose_edit")).unwrap();
        fs::write(
            root.join("capabilities/propose_edit/handler.ts"),
            "export function proposeEdit() {}\n",
        )
        .unwrap();
        let with_handler = granted_capability_ids(&root).unwrap();
        assert!(with_handler.contains("propose_edit"));
        assert_eq!(with_handler.len(), builtins_only.len() + 1);
    }

    /// A Python-shipped handler is grantable on the same terms a TypeScript one
    /// is, and a shared Python module is not.
    ///
    /// The declaration is the folder contract in both frontends:
    /// `capabilities/<id>/handler.py` says the package supplies `<id>`, while
    /// `capabilities/handlers/*.py` is a module a handler may import and states
    /// nothing about what the package can supply.
    #[test]
    fn a_python_shipped_handler_is_grantable_and_a_shared_module_is_not() {
        let tmp = tempdir().unwrap();
        let root = tmp.path().join("python-handler");
        scaffold(&root, "python-handler");
        fs::create_dir_all(root.join("capabilities/handlers")).unwrap();
        fs::write(
            root.join("capabilities/handlers/shared.py"),
            "def shared(): ...\n",
        )
        .unwrap();
        fs::create_dir_all(root.join("capabilities/summarize")).unwrap();
        fs::write(
            root.join("capabilities/summarize/handler.py"),
            "def summarize(): ...\n",
        )
        .unwrap();

        let grantable = granted_capability_ids(&root).unwrap();
        assert!(
            grantable.contains("summarize"),
            "a shipped Python handler supplies its capability: {grantable:?}"
        );
        assert!(
            !grantable.contains("shared"),
            "a shared Python module declares no capability: {grantable:?}"
        );
    }

    /// One capability is supplied by one implementation.
    #[test]
    fn a_capability_shipping_two_language_handlers_is_refused() {
        let tmp = tempdir().unwrap();
        let root = tmp.path().join("two-languages");
        scaffold(&root, "two-languages");
        fs::create_dir_all(root.join("capabilities/summarize")).unwrap();
        fs::write(
            root.join("capabilities/summarize/handler.py"),
            "def summarize(): ...\n",
        )
        .unwrap();
        fs::write(
            root.join("capabilities/summarize/handler.ts"),
            "export function summarize() {}\n",
        )
        .unwrap();

        let error = granted_capability_ids(&root).expect_err("two implementations must be refused");
        assert!(
            error.to_string().contains("summarize"),
            "the refusal names the capability supplied twice: {error}"
        );
    }

    /// The grant set and the dispatchable set are the same set, or the
    /// composition root refuses to bind the package at all.
    #[cfg(feature = "driver")]
    #[test]
    fn a_package_whose_manifest_lost_a_shipped_handler_is_refused() {
        let tmp = tempdir().unwrap();
        let root = tmp.path().join("drifted");
        scaffold(&root, "drifted");
        fs::create_dir_all(root.join("capabilities/propose_edit")).unwrap();
        fs::write(
            root.join("capabilities/propose_edit/handler.ts"),
            "export function proposeEdit() {}\n",
        )
        .unwrap();
        // The handler source is what `granted_capability_ids` reads, so the
        // grant is already claimed; the manifest that would make it
        // dispatchable was never built.
        assert!(
            granted_capability_ids(&root)
                .unwrap()
                .contains("propose_edit")
        );
        seal_agent_integrity_for_test(&root).unwrap();

        let error = admitted_package_handlers(&root).expect_err("the drift must be refused");
        let message = error.to_string();
        assert!(
            message.contains("propose_edit") && message.contains("agent build"),
            "the refusal names the id the grant set claims and how to make it true: {message}"
        );
    }

    /// `agent.toml [permissions]` is the package layer of the resolution stack.
    /// `agent lint` compiles no program, so it states no authored request and
    /// the code layer sits at the `allow` floor: the package may narrow one and
    /// may not decide for a capability nothing grants — there is no request
    /// there for it to narrow.
    #[test]
    fn lint_refuses_a_package_permission_for_a_capability_nothing_grants() {
        let tmp = tempdir().unwrap();
        let root = tmp.path().join("permissions");
        scaffold(&root, "permissions");
        let agent_path = root.join("agent.toml");
        let scaffolded = fs::read_to_string(&agent_path).unwrap();
        let without_permissions = scaffolded
            .split("[permissions]")
            .next()
            .expect("the scaffolded manifest states [permissions] last")
            .to_string();

        for tightened in ["ask", "deny"] {
            fs::write(
                &agent_path,
                format!("{without_permissions}[permissions]\nwrite = \"{tightened}\"\n"),
            )
            .unwrap();
            agent_lint(&root, None, true)
                .unwrap_or_else(|e| panic!("tightening allow to {tightened} resolves: {e}"));
        }

        fs::write(
            &agent_path,
            format!("{without_permissions}[permissions]\nexfiltrate = \"allow\"\n"),
        )
        .unwrap();
        agent_lint(&root, None, true).expect_err("an unrequested override must fail lint");
        let refusals = check_permission_resolution(&load_agent(&root).unwrap(), &BTreeSet::new());
        assert!(
            refusals
                .iter()
                .any(|refusal| refusal.contains("never requested")),
            "expected an unrequested-override refusal, got: {refusals:?}"
        );

        // An org supplying that capability is exactly what makes the decision
        // legitimate, which is the whole remaining job of `agent lint --org`.
        let org_globals = BTreeSet::from(["exfiltrate".to_string()]);
        assert!(
            check_permission_resolution(&load_agent(&root).unwrap(), &org_globals).is_empty(),
            "an org-global capability must accept a package decision"
        );
    }

    /// The other arm of the lattice, reachable only once a caller knows what
    /// the program itself requested.
    ///
    /// With no authored request the code layer is a maximally wide floor, and
    /// nothing can widen what is already widest — `write = "allow"` resolves
    /// clean. The same package against a program whose source asked for a gate
    /// on `write` is a package handing back authority the program declined, and
    /// it fails closed. `compile-service-canonical` is the caller that knows the
    /// difference, because it has just lowered the program.
    #[test]
    fn a_package_may_not_widen_what_the_program_requested() {
        let tmp = tempdir().unwrap();
        let root = tmp.path().join("widening");
        scaffold(&root, "widening");
        let agent_path = root.join("agent.toml");
        let scaffolded = fs::read_to_string(&agent_path).unwrap();
        let without_permissions = scaffolded
            .split("[permissions]")
            .next()
            .expect("the scaffolded manifest states [permissions] last")
            .to_string();
        fs::write(
            &agent_path,
            format!("{without_permissions}[permissions]\nwrite = \"allow\"\n"),
        )
        .unwrap();

        resolve_package_permission_layers(&root, &LayerDecisions::new())
            .expect("nothing can widen an unstated request");

        let authored = LayerDecisions::from([(
            "write".to_string(),
            PermissionDecision::ask("Writes files on the host."),
        )]);
        let error = resolve_package_permission_layers(&root, &authored)
            .expect_err("a package may not widen an authored request")
            .to_string();
        assert!(
            error.contains("may only tighten"),
            "expected a widening refusal, got: {error}"
        );

        let resolved = resolve_package_permission_layers(
            &root,
            &LayerDecisions::from([(
                "read".to_string(),
                PermissionDecision::ask("Reads whatever the model asks for."),
            )]),
        )
        .expect("an authored request no package layer touches resolves");
        assert_eq!(
            resolved.get("read"),
            Some(&PermissionDecision::ask(
                "Reads whatever the model asks for."
            )),
            "an unnarrowed authored request is the resolved decision, not a blanket allow"
        );
    }

    /// The skill-discovery ids are grantable *because* something implements
    /// them, and the ids that were removed for having no handler are still
    /// refused.
    ///
    /// This is the asymmetry the whole re-add rests on. `list_skills`,
    /// `search_skills`, and `read_skill` are back in the built-in allowlist
    /// only because `apxm_capability::builtins::skills` supplies handlers for
    /// them, so a package may decide about them. `read_local_skill` and
    /// `list_local_skills` are the exact ids that were once allowlisted with
    /// nothing behind them — the state where `agent lint` accepted a package
    /// declaring a capability that could never execute. A package naming one
    /// today fails, and it fails for the right reason: nothing grants it.
    #[test]
    fn lint_grants_the_implemented_skill_capabilities_and_still_refuses_the_unimplemented_ones() {
        let tmp = tempdir().unwrap();
        let root = tmp.path().join("skills");
        scaffold(&root, "skills");
        let agent_path = root.join("agent.toml");
        let scaffolded = fs::read_to_string(&agent_path).unwrap();
        let without_permissions = scaffolded
            .split("[permissions]")
            .next()
            .expect("the scaffolded manifest states [permissions] last")
            .to_string();

        let grantable = granted_capability_ids(&root).unwrap();
        for implemented in ["list_skills", "search_skills", "read_skill"] {
            assert!(
                grantable.contains(implemented),
                "'{implemented}' has a handler, so a package may decide about it"
            );
            fs::write(
                &agent_path,
                format!("{without_permissions}[permissions]\n{implemented} = \"ask\"\n"),
            )
            .unwrap();
            agent_lint(&root, None, true)
                .unwrap_or_else(|e| panic!("deciding about {implemented} must resolve: {e}"));
        }

        for unimplemented in ["read_local_skill", "list_local_skills"] {
            assert!(
                !grantable.contains(unimplemented),
                "'{unimplemented}' has no handler and must not be grantable"
            );
            fs::write(
                &agent_path,
                format!("{without_permissions}[permissions]\n{unimplemented} = \"allow\"\n"),
            )
            .unwrap();
            agent_lint(&root, None, true).unwrap_err();
            let refusals =
                check_permission_resolution(&load_agent(&root).unwrap(), &BTreeSet::new());
            assert!(
                refusals
                    .iter()
                    .any(|refusal| refusal.contains("never requested")),
                "a skill capability with no handler must fail lint, got: {refusals:?}"
            );
        }
    }

    /// A capability's decision and the reason declared beside it survive into
    /// the one place a decision is now stated.
    #[test]
    fn a_declared_decision_keeps_the_reason_declared_beside_it() {
        let tmp = tempdir().unwrap();
        let root = tmp.path().join("reasoned");
        scaffold(&root, "reasoned");
        let agent_path = root.join("agent.toml");
        let scaffolded = fs::read_to_string(&agent_path).unwrap();
        let without_permissions = scaffolded
            .split("[permissions]")
            .next()
            .expect("the scaffolded manifest states [permissions] last")
            .to_string();
        fs::write(
            &agent_path,
            format!(
                "{without_permissions}[permissions]\n                 write = {{ decision = \"ask\", reason = \"Writes files on the host.\" }}\n"
            ),
        )
        .unwrap();

        let pkg = load_agent(&root).unwrap();
        let resolved = resolve_permission_layers(
            &pkg.agent,
            &granted_capability_ids(&root).unwrap(),
            &LayerDecisions::new(),
        )
        .unwrap();
        assert_eq!(
            resolved.get("write"),
            Some(&PermissionDecision::ask("Writes files on the host."))
        );
        agent_lint(&root, None, true).expect("a reasoned decision resolves");
    }

    #[test]
    fn new_resolves_example_names_and_template_paths_generically() {
        let coder = resolve_agent_template_dir("coder").expect("named example resolves");
        assert_eq!(coder, example_agent_dir("coder"));

        let tmp = tempdir().unwrap();
        let template = tmp.path().join("custom-template");
        fs::create_dir_all(&template).unwrap();
        fs::write(
            template.join("agent.toml"),
            "id = \"template\"\nversion = \"0.1.0\"\n",
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
        assert!(!destination.join("integrity.toml").exists());
    }

    /// The decision is about a capability id, so both languages carry it.
    #[test]
    fn a_manifest_carries_the_resolved_permission_decision_in_either_language() {
        let tmp = tempdir().unwrap();
        fs::create_dir_all(tmp.path().join("capabilities/handlers")).unwrap();
        let descriptor = |name: &str, hash: char, language: HandlerLanguage| HandlerDescriptor {
            kind: HandlerKind::Tool,
            language,
            handler_id: format!("sha256:{}", hash.to_string().repeat(64)),
            module: format!("capabilities/{name}/handler"),
            qualname: "run".to_string(),
            name: name.to_string(),
            source: HandlerSource {
                artifact_path: format!("handlers/{name}.{}", agent_packaging(language).extension),
                content: "def run(): ...\n".to_string(),
            },
            description: None,
            schema: Some(serde_json::json!({})),
            read_only: None,
            requires_approval: None,
        };
        let mut manifest = HandlerManifest::new(vec![
            descriptor("read_tool", 'a', HandlerLanguage::TypeScript),
            descriptor("write_tool", 'b', HandlerLanguage::Python),
        ]);
        let resolved = BTreeMap::from([
            ("read_tool".to_string(), PermissionDecision::allow()),
            (
                "write_tool".to_string(),
                PermissionDecision::ask("writes the host"),
            ),
        ]);

        apply_resolved_permissions_to_tools_manifest(&mut manifest, &resolved)
            .expect("policy join");
        write_tools_manifest(tmp.path(), &manifest).expect("manifest write");

        let loaded = load_tools_manifest(tmp.path()).expect("manifest reload");
        let by_name = loaded
            .handlers
            .iter()
            .map(|entry| (entry.name.as_str(), entry))
            .collect::<BTreeMap<_, _>>();
        assert_eq!(by_name["read_tool"].requires_approval, Some(false));
        assert_eq!(by_name["write_tool"].requires_approval, Some(true));

        // A handler for a capability the resolution denies, and a handler for a
        // capability nothing grants, are both refused rather than emitted.
        for (label, decisions) in [
            (
                "denied",
                BTreeMap::from([(
                    "read_tool".to_string(),
                    PermissionDecision::deny("no reads"),
                )]),
            ),
            ("ungranted", BTreeMap::new()),
        ] {
            let mut refused =
                HandlerManifest::new(vec![descriptor("read_tool", 'a', HandlerLanguage::Python)]);
            assert!(
                apply_resolved_permissions_to_tools_manifest(&mut refused, &decisions).is_err(),
                "a {label} capability must not be emitted as an executable handler"
            );
        }
    }

    #[test]
    fn load_tools_manifest_rejects_non_conforming_manifest() {
        let tmp = tempdir().unwrap();
        let handlers_dir = tmp.path().join("capabilities/handlers");
        fs::create_dir_all(&handlers_dir).unwrap();
        // A tool descriptor whose artifact_path escapes the artifact onto a
        // build-host path must be rejected at admission:
        // HandlerManifest::validate() fails closed on a non-conforming
        // manifest instead of silently trusting it.
        fs::write(
            handlers_dir.join("tools.json"),
            serde_json::json!({
                "version": "apxm.handler-manifest",
                "handlers": [
                    {
                        "kind": "tool",
                        "language": "typescript",
                        "handler_id": format!("sha256:{}", "a".to_string().repeat(64)),
                        "module": "capabilities/echo/handler",
                        "qualname": "echo",
                        "name": "echo",
                        "source": {
                            "artifact_path": "/tmp/build-host/echo.mjs",
                            "content": "export function echo() {}\n",
                        },
                        "schema": {"type": "object"},
                    }
                ],
            })
            .to_string(),
        )
        .unwrap();

        let err = load_tools_manifest(tmp.path())
            .expect_err("a build-host artifact path must not be admitted");
        assert!(
            err.to_string().contains("Invalid"),
            "expected an admission-boundary rejection, got: {err}"
        );
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
    fn lint_ignores_local_node_build_sidecars() {
        let tmp = tempdir().unwrap();
        let root = tmp.path().join("sidecars");
        scaffold(&root, "sidecars");
        fs::create_dir_all(root.join("node_modules/example")).unwrap();
        fs::create_dir_all(root.join("dist/src")).unwrap();
        fs::write(root.join("node_modules/example/index.js"), "export {};").unwrap();
        fs::write(root.join("dist/src/main.js"), "export {};").unwrap();

        agent_lint(&root, None, true)
            .expect("local build sidecars do not violate the folder contract");
    }

    /// The folder contract recognizes `skills/`, now that a producer exists.
    ///
    /// It was closed while nothing could author such a skill: `agent lint`
    /// would have accepted a directory the machine could do nothing with, and
    /// `walk_recognized_files` would have hashed it into every package's
    /// integrity chain on behalf of a producer that did not exist. The
    /// ordering is the one the built-in capability allowlist follows — the
    /// thing that consumes the path lands first, then the path is recognized.
    /// Both halves are here now: the `Skill` marker declares one and
    /// `Skill(...).load()` reads it through `read_skill`.
    ///
    /// The instruction document is the one file name `apxm.package-local-skill`
    /// admits; everything else a skill carries is a resource, and nesting is
    /// allowed there because supporting material has its own shape.
    #[test]
    fn agent_package_recognizes_local_skill_resources() {
        assert!(recognized_relpath("skills/review/SKILL.md"));
        assert!(recognized_relpath("skills/review/resources/guide.md"));
        assert!(recognized_relpath("skills/review/resources/deep/table.csv"));
        // A skill is its instruction document plus resources, not a second
        // place to put arbitrary package content.
        assert!(!recognized_relpath("skills/review/notes.md"));
        assert!(!recognized_relpath("skills/SKILL.md"));
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
        fs::write(root.join("skills/persona/SKILL.md"), "tampered content\n").unwrap();
        let tampered_files = digest_recognized_files(&root).expect("recompute after tamper");
        let persona_link = integrity
            .chain
            .iter()
            .find(|l| l.path == "skills/persona/SKILL.md")
            .expect("the persona skill is in the chain");
        let mut preimage = String::new();
        preimage.push_str(&persona_link.prev_hash);
        preimage.push_str(&persona_link.path);
        preimage.push_str(&tampered_files["skills/persona/SKILL.md"]);
        assert_ne!(
            sha256_hex(preimage.as_bytes()),
            persona_link.hash,
            "tampering with a hashed file must invalidate its chain link"
        );
        // The gate `check-agent-packages` runs is the same one, so the tamper
        // that breaks a link also fails verification.
        verify_agent_integrity(&root)
            .expect_err("a package edited after its last build must fail verification");
    }

    /// `agent verify` refuses a package that grew a file the folder contract
    /// does not recognize, even when every hashed file is untouched.
    #[test]
    fn verify_refuses_a_package_carrying_a_retired_manifest() {
        let tmp = tempdir().unwrap();
        let root = tmp.path().join("legacy");
        scaffold(&root, "legacy");
        agent_build(&root, true).expect("build ok");
        verify_agent_integrity(&root).expect("a freshly built package verifies");

        fs::create_dir_all(root.join("capabilities")).unwrap();
        fs::write(
            root.join("capabilities/capabilities.toml"),
            "[[capability]]\nid = \"read\"\n",
        )
        .unwrap();

        let error = verify_agent_integrity(&root)
            .expect_err("a package still carrying capabilities.toml must fail verification");
        assert!(
            error.to_string().contains("capabilities/capabilities.toml"),
            "expected the retired manifest to be named, got: {error}"
        );
        agent_lint(&root, None, true)
            .expect_err("a package still carrying capabilities.toml must fail lint");
    }

    // Spawns the package's Python entry, which imports the apxm_program frontend;
    // it runs only where that package is installed on the subprocess path (as the
    // packed-frontend gate arranges), not under the bare CLI test harness.
    #[ignore = "requires the apxm_program frontend installed on the subprocess path"]
    #[cfg(feature = "driver")]
    #[test]
    fn studio_style_source_package_builds_from_its_compile_entry_alone() {
        let tmp = tempdir().unwrap();
        let root = tmp.path().join("studio-generated");
        fs::create_dir_all(root.join("python")).unwrap();
        fs::write(
            root.join("agent.toml"),
            "id = \"studio-generated\"\n\
             version = \"0.1.0\"\n\
             schema_version = \"apxm.agent\"\n\n\
             [compile]\n\
             entry = \"python/main.py\"\n\
             frontend = \"python\"\n",
        )
        .unwrap();
        fs::write(
            root.join("python/main.py"),
            "from typing import TypedDict\n\
             \n\
             from apxm_program import Agent, Model\n\
             \n\
             \n\
             class StudioInput(TypedDict):\n\
             \x20\x20\x20\x20message: str\n\
             \n\
             \n\
             class StudioOutput(TypedDict):\n\
             \x20\x20\x20\x20message: str\n\
             \n\
             \n\
             StudioModel = Model[StudioInput, StudioOutput](\"model.target\")\n\
             \n\
             \n\
             @Agent(input=StudioInput, output=StudioOutput)\n\
             async def StudioGenerated(agent, request):\n\
             \x20\x20\x20\x20while request[\"message\"] != \"\":\n\
             \x20\x20\x20\x20\x20\x20\x20\x20reply = await StudioModel(request)\n\
             \x20\x20\x20\x20\x20\x20\x20\x20request = await agent.yield_(reply)\n\
             \x20\x20\x20\x20raise ValueError(\"missing studio input\")\n\n\
             if __name__ == \"__main__\":\n\
             \x20\x20\x20\x20print(StudioGenerated.canonical_air(), end=\"\")\n",
        )
        .unwrap();
        agent_build(&root, true).expect("a generated source package must build");
        let agent: AgentToml = read_toml(&root.join("agent.toml")).unwrap();
        assert_eq!(
            agent
                .compile
                .as_ref()
                .and_then(|compile| compile.entry.as_deref()),
            Some("python/main.py")
        );

        let air =
            super::super::compile_service_canonical::emit_canonical_air_from_agent(&root, None)
                .expect("canonical compile-service must compile the package-level program entry");
        assert!(air.contains("\"schema_version\":\"apxm.air\""));
        assert!(air.contains("\"op\":\"model.call\""));
    }

    /// Sync regenerates the handler manifest and nothing else. It used to
    /// rewrite `agent.toml`'s capability array on every run, which is how a
    /// deleted inventory would have come straight back.
    #[test]
    fn sync_leaves_the_authored_manifest_byte_for_byte() {
        let tmp = tempdir().unwrap();
        let root = tmp.path().join("catalogue");
        scaffold(&root, "catalogue");
        let before = fs::read_to_string(root.join("agent.toml")).expect("agent manifest");

        agent_sync(&root, true).expect("sync ok");
        agent_sync(&root, true).expect("sync is idempotent");

        let after = fs::read_to_string(root.join("agent.toml")).expect("agent manifest");
        assert_eq!(before, after, "sync must not rewrite the authored manifest");
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
        // A package's skills install with it. They are recognized package
        // content now, hashed into the same chain as everything else, so an
        // installed package carries the instructions its program loads.
        assert!(dest.join("skills/persona/SKILL.md").is_file());

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
    // apxm.agent contract conformance
    // -----------------------------------------------------------------

    /// The published `apxm.agent` vectors, decoded from their checked-in bytes.
    fn agent_vectors() -> Vec<serde_json::Value> {
        let text = include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../../contracts/vectors/apxm.agent.json"
        ));
        serde_json::from_str::<Vec<serde_json::Value>>(text)
            .expect("the published apxm.agent vectors are a JSON array")
    }

    /// Materialize one vector as a package on disk and run the real
    /// `agent lint` over it.
    ///
    /// The vector's `files` map is the package layout, so every path it names
    /// becomes a file; `agent.toml` is written from the vector's manifest. This
    /// is the whole admission the CLI performs, held against the same document
    /// the published schema judges.
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
        let manifest = toml::to_string(&vector["agent"]).map_err(|error| error.to_string())?;
        fs::write(root.join("agent.toml"), manifest).map_err(|error| error.to_string())?;

        agent_lint(&root, None, true).map_err(|error| error.to_string())
    }

    /// Every vector's verdict matches what `agent lint` decides about the same
    /// package.
    ///
    /// This is the drift gate the most author-facing surface in the system never
    /// had: a key the schema stops naming, a path it stops recognizing, or a
    /// rule the CLI stops enforcing, fails here rather than in some author's
    /// package months later.
    #[test]
    fn agent_vectors_match_the_lint_path() {
        for vector in agent_vectors() {
            let name = vector["name"].as_str().expect("vector name");
            let expected = vector["expected_valid"].as_bool().expect("expected_valid");
            let verdict = lint_admits_vector(&vector["input"]);
            assert_eq!(
                verdict.is_ok(),
                expected,
                "vector '{name}' expected valid={expected} but agent lint returned {verdict:?}",
            );
        }
    }

    /// The digests the integrity chain records satisfy the shape the contract
    /// publishes for them, so the producer and the schema cannot drift.
    #[test]
    fn produced_digests_satisfy_the_published_digest_pattern() {
        let schema: serde_json::Value =
            serde_json::from_str(AGENT_SCHEMA_JSON).expect("the embedded contract is valid JSON");
        let pattern = schema["$defs"]["FileDigest"]["pattern"]
            .as_str()
            .expect("FileDigest pattern");
        let published = Regex::new(pattern).expect("the published digest pattern compiles");
        assert!(published.is_match(&sha256_hex(b"apxm.agent")));
        assert!(published.is_match(GENESIS_HASH));
    }

    /// Every path the published contract recognizes, and only those.
    ///
    /// `recognized_relpath` is compiled from the schema's own patterns, so this
    /// asserts the derivation actually took: the retired manifests are refused
    /// and the surviving layout is admitted.
    #[test]
    fn the_recognized_folder_contract_is_the_published_one() {
        for recognized in [
            "agent.toml",
            "integrity.toml",
            "README.md",
            "package.json",
            "tsconfig.json",
            "capabilities/handlers/tools.json",
            "capabilities/handlers/shared.ts",
            "capabilities/edit/handler.ts",
            "prompts/persona.md",
            "skills/review/SKILL.md",
            "skills/review/resources/checklist.md",
            "python/agent.py",
            "src/main.ts",
            "src/nested/deep/module.ts",
            "examples/basic.md",
            "tests/acceptance.py",
            "shared/notes.txt",
        ] {
            assert!(
                recognized_relpath(recognized),
                "the published contract must recognize {recognized}"
            );
        }
        for retired in [
            "hierarchy.toml",
            "capabilities/capabilities.toml",
            "capabilities/permissions.toml",
            "capabilities/read/capability.toml",
            "capabilities/read/permission.toml",
            "capabilities/edit/nested/handler.ts",
            "prompts/persona.txt",
            "skills/review/notes.md",
            "not-a-real-file.txt",
        ] {
            assert!(
                !recognized_relpath(retired),
                "the published contract must not recognize {retired}"
            );
        }
    }

    /// The schema names the version constant the manifest check enforces.
    #[test]
    fn the_schema_version_constant_is_read_from_the_published_contract() {
        let schema: serde_json::Value =
            serde_json::from_str(AGENT_SCHEMA_JSON).expect("the embedded contract is valid JSON");
        assert_eq!(schema["$id"].as_str(), Some(AGENT_SCHEMA));
        assert_eq!(
            schema["$defs"]["AgentManifest"]["properties"]["schema_version"]["const"].as_str(),
            Some(AGENT_SCHEMA),
            "the schema_version constant drifted from the schema `const`",
        );
    }

    /// Every checked-in agent package satisfies the folder contract it names.
    #[test]
    fn checked_in_packages_satisfy_the_published_folder_contract() {
        let repository_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..");
        for package in [
            "examples/agents/conversational",
            "examples/agents/coder",
            "examples/agents/skilled",
            "crates/compiler/frontend/python/tests_program/fixtures/canonical_session_agent",
        ] {
            let root = repository_root.join(package);
            let unrecognized = find_unrecognized_files(&root)
                .unwrap_or_else(|error| panic!("walk {package}: {error}"));
            assert!(
                unrecognized.is_empty(),
                "{package} carries paths outside the published folder contract: {unrecognized:?}"
            );
            let pkg = load_agent(&root).unwrap_or_else(|error| panic!("load {package}: {error}"));
            assert_eq!(pkg.agent.schema_version.as_deref(), Some(AGENT_SCHEMA));
        }
    }
}
