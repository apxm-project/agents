//! `apxm integration new|lint|install` — the toolchain for the
//! canonical integration-package folder format (`apxm.integration-package.v1`).
//!
//! This module is the integration-level sibling of [`super::agent`]
//! and [`super::org`]: same module organization, same
//! manifest-projection-from-schema approach (hand ported into Rust structs
//! rather than loading the JSON schema at runtime), same lint reporting
//! style, same install-to-`APXM_HOME` pattern. The contract schema only
//! formalizes the *wrapper* shape (six named files, packed as raw text) —
//! the per-file internal grammar below is reverse-engineered from the eleven
//! real packs under `studio/integrations/` (slack, generic-http, …) plus
//! `docs/guides/integrations.md`.
//!
//! The six files are the pre-existing five (`integration.toml`, `provider.toml`,
//! `capabilities.toml`, `triggers.toml`, `connector.toml`) **plus
//! `permissions.toml`** — the policy half of every declared capability,
//! using the same permission vocabulary
//! (`crates/machine/contracts/src/types/capability/permission_vocabulary.rs`):
//! `operation_class`, `risk_level`, `approval_posture`, `scope`,
//! `credential_scope`, `audit_payload`. A capability entry with no policy
//! entry fails lint (the same "joined capability" rule already enforced for
//! agent/org packages — one capability grammar across all three
//! pack families).
//!
//! ## Fan-out
//!
//! Each consumer's own source shows that auth-ms, apxm-server, apxm-os, and
//! Studio do **not** register integrations via any push/HTTP API — they
//! all discover the catalog by *scanning a shared filesystem root* at their
//! own startup/reindex time:
//!
//! - `auth/crates/auth/src/providers/registry.rs::integration_roots` scans
//!   `$APXM_INTEGRATIONS_ROOT`, then `$APXM_WORKSPACE_ROOT/integrations`.
//! - `server/crates/server/src/startup.rs::integration_capability_roots`
//!   scans the same two env-derived roots, **plus every
//!   `ApxmPaths::integrations_dirs()` root** (which includes
//!   `$APXM_HOME/integrations` as its lowest-precedence fallback — see
//!   `crates/machine/contracts/src/paths.rs`).
//! - `os/crates/os-supervisor/src/control.rs::control_integrations_list`
//!   scans `<workspace_root>/integrations` and `$APXM_INTEGRATIONS_ROOT`.
//! - `studio/crates/studio/src/integration.rs::integrations_scan_roots`
//!   scans `$APXM_INTEGRATIONS_ROOT`, then `$APXM_WORKSPACE_ROOT/integrations`,
//!   then its own bundled `integrations/` tree.
//! - `studio/scripts/stack-lib.sh::sync_integrations` (run by `dekk studio
//!   stack seed` and automatically before `stack up`'s apxm-server) is the
//!   existing precedent for this exact fan-out: it copies the bundled
//!   integration folders into `$APXM_WORKSPACE_ROOT/integrations` so "Studio,
//!   Server, OS, and Auth scanners share one runtime catalog" (its own
//!   comment).
//!
//! No consumer exposes (or is guaranteed to have running/reachable) a
//! network registration endpoint for this from a CLI context. So `integration
//! install` fans out the pack the same way `sync_integrations` already does:
//! it always copies to `APXM_HOME/integrations/<id>/` (the /
//! convention, and one of `server`'s own scan roots), and — best-effort, so a
//! missing/unreachable shared workspace never fails the install — copies the
//! same copy to `$APXM_WORKSPACE_ROOT/integrations/<id>/` when
//! `APXM_WORKSPACE_ROOT` is set, which is the root auth-ms, apxm-os, and
//! Studio actually scan. A failure to copy is a warning, not an error: the
//! `APXM_HOME` copy is always the source of truth this command guarantees.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow, bail};
use serde::{Deserialize, Serialize};

use super::agent::copy_dir_recursive;
use super::implementations::{Status, print_section_header, print_status_line};

// ---------------------------------------------------------------------
// On-disk manifest shapes (apxm.integration-package.v1 projections)
// ---------------------------------------------------------------------

/// Projection of `integration.toml` — the pack manifest.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IntegrationToml {
    pub schema_version: i64,
    #[serde(default)]
    pub profile_version: Option<String>,
    pub integration_id: String,
    pub provider: String,
    pub version: String,
    #[serde(default)]
    pub display_name: Option<String>,
    #[serde(default)]
    pub category: Option<String>,
    pub maturity: String,
    pub files: FilesToml,
    #[serde(default)]
    pub capabilities: Option<CapabilitiesRefToml>,
}

/// `integration.toml`'s `[files]` table: sibling file references (relative
/// paths within the pack). `permissions` is /'s addition — the
/// existing eleven packs' `integration.toml` predate it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FilesToml {
    pub provider: String,
    pub connector: String,
    pub capabilities: String,
    pub triggers: String,
    #[serde(default)]
    pub permissions: Option<String>,
}

/// `integration.toml`'s `[capabilities]` table: the declared action/trigger
/// id lists (advisory cross-reference; `capabilities.toml`/`triggers.toml`
/// remain the source of truth checked by lint).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CapabilitiesRefToml {
    #[serde(default)]
    pub actions: Vec<String>,
    #[serde(default)]
    pub triggers: Vec<String>,
}

/// `capabilities.toml` — array of provider action definitions. Shape taken
/// verbatim from the real packs (`studio/integrations/slack/capabilities.toml`,
/// `.../generic-http/capabilities.toml`): advisory flags only
/// (`read_only`/`requires_auth`), no permission policy; that lives in
/// `permissions.toml`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CapabilitiesToml {
    #[serde(default, rename = "capability")]
    pub capability: Vec<CapabilityEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CapabilityEntry {
    pub id: String,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub kind: Option<String>,
    #[serde(default)]
    pub read_only: bool,
    #[serde(default)]
    pub requires_auth: bool,
    #[serde(default)]
    pub method: Option<String>,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub version: Option<i64>,
    #[serde(default)]
    pub schema: Option<String>,
}

/// `permissions.toml` — one policy entry per joined capability id. Field names
/// use the permission
/// vocabulary (`apxm.permission-policy.v1`) exactly:
/// `operation_class`/`risk_level`/`approval_posture`/`scope`/
/// `credential_scope`/`audit_payload` — see
/// `crates/machine/contracts/src/types/capability/permission_vocabulary.rs`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PermissionsToml {
    #[serde(default, rename = "permission")]
    pub permission: Vec<PermissionEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PermissionEntry {
    pub capability: String,
    #[serde(default)]
    pub operation_class: Option<String>,
    #[serde(default)]
    pub risk_level: Option<String>,
    pub approval_posture: String,
    #[serde(default)]
    pub scope: Option<String>,
    #[serde(default)]
    pub credential_scope: Option<String>,
    #[serde(default)]
    pub audit_payload: Option<String>,
}

/// Write-capable capabilities default to
/// approval-required; read-only capabilities default to auto (no gate).
/// `auto` is the `ApprovalPosture::Auto` value; any other posture
/// (`confirm`, `dual_control`, `external_signoff`) counts as
/// "approval-required" for lint purposes.
const POSTURE_AUTO: &str = "auto";
const POSTURE_APPROVAL_REQUIRED_DEFAULT: &str = "confirm";

fn default_posture_for(read_only: bool) -> &'static str {
    if read_only {
        POSTURE_AUTO
    } else {
        POSTURE_APPROVAL_REQUIRED_DEFAULT
    }
}

fn default_risk_for(read_only: bool) -> &'static str {
    if read_only { "low" } else { "medium" }
}

fn default_operation_class_for(read_only: bool) -> &'static str {
    if read_only { "read" } else { "write" }
}

// ---------------------------------------------------------------------
// integration new
// ---------------------------------------------------------------------

pub fn integration_command(action: super::IntegrationAction, json_output: bool) -> Result<()> {
    match action {
        super::IntegrationAction::New {
            id,
            path,
            display_name,
        } => integration_new(&id, path, display_name, json_output),
        super::IntegrationAction::Lint { path } => integration_lint(&path, json_output),
        super::IntegrationAction::Install { path, force } => {
            integration_install(&path, force, json_output)
        }
    }
}

fn default_integration_root(id: &str) -> PathBuf {
    PathBuf::from("integrations").join(id)
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

fn integration_new(
    id: &str,
    path: Option<PathBuf>,
    display_name: Option<String>,
    json_output: bool,
) -> Result<()> {
    if id.trim().is_empty() {
        bail!("integration id must not be empty");
    }
    let root = path.unwrap_or_else(|| default_integration_root(id));
    if root.exists() && fs::read_dir(&root)?.next().is_some() {
        bail!(
            "destination '{}' already exists and is not empty",
            root.display()
        );
    }

    let display_name = display_name.unwrap_or_else(|| titleize(id));
    let probe_cap = format!("{id}.probe");
    let send_cap = format!("{id}.send");

    // integration.toml — the manifest. `[files]` includes `permissions`
    // ('s addition), unlike the eleven pre-INT packs.
    let integration_toml = format!(
        "schema_version = 1\n\
         profile_version = \"integration/v1\"\n\
         integration_id = \"{id}\"\n\
         provider = \"{id}\"\n\
         version = \"0.1.0\"\n\
         display_name = \"{display_name}\"\n\
         category = \"data\"\n\
         maturity = \"draft\"\n\n\
         [files]\n\
         provider = \"provider.toml\"\n\
         connector = \"connector.toml\"\n\
         capabilities = \"capabilities.toml\"\n\
         triggers = \"triggers.toml\"\n\
         permissions = \"permissions.toml\"\n\n\
         [capabilities]\n\
         actions = [\"{probe_cap}\", \"{send_cap}\"]\n\
         triggers = []\n"
    );
    write_new_file(&root.join("integration.toml"), &integration_toml)?;

    // provider.toml — auth facts only, never secrets.
    let provider_toml = format!(
        "display_name = \"{display_name}\"\n\
         auth_kind = \"none\"\n\
         category = \"data\"\n\
         # OAuth providers set authorize_url/token_url/api_base/default_scopes\n\
         # and a [webhook_secret] table here instead. See integrations/slack/\n\
         # provider.toml for the reference oauth2 shape.\n"
    );
    write_new_file(&root.join("provider.toml"), &provider_toml)?;

    // capabilities.toml — one read-only and one write example, matching the
    // shape of the real packs (studio/integrations/{slack,generic-http}).
    let capabilities_toml = format!(
        "[[capability]]\n\
         id = \"{probe_cap}\"\n\
         name = \"{display_name}: probe\"\n\
         kind = \"provider\"\n\
         read_only = true\n\
         requires_auth = false\n\
         method = \"GET\"\n\
         url = \"https://example.com\"\n\
         version = 1\n\
         schema = \"\"\"{{\"type\":\"object\",\"properties\":{{}}}}\"\"\"\n\n\
         [[capability]]\n\
         id = \"{send_cap}\"\n\
         name = \"{display_name}: send\"\n\
         kind = \"provider\"\n\
         read_only = false\n\
         requires_auth = true\n\
         method = \"POST\"\n\
         url = \"https://example.com\"\n\
         version = 1\n\
         schema = \"\"\"{{\"type\":\"object\",\"required\":[\"message\"],\"properties\":{{\"message\":{{\"type\":\"string\"}}}}}}\"\"\"\n"
    );
    write_new_file(&root.join("capabilities.toml"), &capabilities_toml)?;

    // permissions.toml — the policy half, one [[permission]] per
    // capabilities.toml entry: auto
    // for read_only, approval-required (confirm) otherwise.
    let permissions_toml = format!(
        "# One [[permission]] per capabilities.toml entry (by id) — the policy\n\
         # half of every capability.\n\
         # A capability with no matching entry here fails `apxm integration lint`.\n\
         [[permission]]\n\
         capability = \"{probe_cap}\"\n\
         operation_class = \"{}\"\n\
         risk_level = \"{}\"\n\
         approval_posture = \"{}\"\n\
         scope = \"workspace\"\n\
         credential_scope = \"none\"\n\
         audit_payload = \"metadata\"\n\n\
         [[permission]]\n\
         capability = \"{send_cap}\"\n\
         operation_class = \"{}\"\n\
         risk_level = \"{}\"\n\
         approval_posture = \"{}\"\n\
         scope = \"workspace\"\n\
         credential_scope = \"connection\"\n\
         audit_payload = \"metadata\"\n",
        default_operation_class_for(true),
        default_risk_for(true),
        default_posture_for(true),
        default_operation_class_for(false),
        default_risk_for(false),
        default_posture_for(false),
    );
    write_new_file(&root.join("permissions.toml"), &permissions_toml)?;

    // triggers.toml — no bundled triggers by default (matches
    // integrations/generic-http/triggers.toml).
    write_new_file(
        &root.join("triggers.toml"),
        "# One [[trigger]] per webhook/channel/cron cue template. See\n\
         # integrations/slack/triggers.toml for the reference shape. Empty by\n\
         # default; a channel trigger must name a live `adapter = \"...\"` before\n\
         # this pack can reach maturity = \"production\".\n",
    )?;

    // connector.toml — canvas UX metadata for the scaffolded probe action.
    let connector_toml = format!(
        "schema_version = 1\n\
         profile_version = \"connector-plugin/v1\"\n\
         app_id = \"{id}\"\n\
         provider = \"{id}\"\n\n\
         [display]\n\
         name = \"{display_name}\"\n\
         category = \"data\"\n\
         description = \"{display_name} integration.\"\n\n\
         [[operation]]\n\
         id = \"probe\"\n\
         kind = \"action\"\n\
         runtime_node = \"tool\"\n\
         source = \"capabilities.toml:{probe_cap}\"\n\
         title = \"Probe\"\n"
    );
    write_new_file(&root.join("connector.toml"), &connector_toml)?;

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
        print_section_header("Integration Scaffolded");
        print_status_line(id, Status::Ok, &root.display().to_string());
        println!();
        println!("Next steps:");
        println!("  apxm integration lint {}", root.display());
        println!("  apxm integration install {}", root.display());
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

struct LoadedIntegration {
    #[allow(dead_code)]
    root: PathBuf,
    integration: IntegrationToml,
    capabilities: CapabilitiesToml,
    permissions: PermissionsToml,
}

/// The six required files, per `apxm.integration-package.v1#/properties/files`.
const REQUIRED_FILES: &[&str] = &[
    "integration.toml",
    "provider.toml",
    "capabilities.toml",
    "permissions.toml",
    "triggers.toml",
    "connector.toml",
];

fn load_integration(root: &Path) -> Result<LoadedIntegration> {
    if !root.is_dir() {
        bail!("'{}' is not a directory", root.display());
    }
    for required in REQUIRED_FILES {
        if !root.join(required).is_file() {
            bail!("missing required file: {}", root.join(required).display());
        }
    }

    let integration: IntegrationToml = read_toml(&root.join("integration.toml"))?;
    let capabilities: CapabilitiesToml = read_toml(&root.join("capabilities.toml"))?;
    let permissions: PermissionsToml = read_toml(&root.join("permissions.toml"))?;

    // provider.toml, triggers.toml, connector.toml must parse as valid TOML
    // (their shape is UX/auth metadata not otherwise checked by this lint).
    let _: toml::Value = read_toml(&root.join("provider.toml"))?;
    let _: toml::Value = read_toml(&root.join("triggers.toml"))?;
    let _: toml::Value = read_toml(&root.join("connector.toml"))?;

    Ok(LoadedIntegration {
        root: root.to_path_buf(),
        integration,
        capabilities,
        permissions,
    })
}

// ---------------------------------------------------------------------
// integration lint
// ---------------------------------------------------------------------

fn semver_like(version: &str) -> bool {
    let core = version.split(['-', '+']).next().unwrap_or(version);
    let parts: Vec<&str> = core.split('.').collect();
    parts.len() == 3
        && parts
            .iter()
            .all(|p| !p.is_empty() && p.chars().all(|c| c.is_ascii_digit()))
}

/// Structural checks for `apxm.integration-package.v1`'s required
/// fields and the folder contract's own identity rules.
fn check_schema_shape(pkg: &LoadedIntegration) -> Vec<String> {
    let mut errors = Vec::new();

    if pkg.integration.integration_id.trim().is_empty() {
        errors.push("integration.toml: integration_id must not be empty".to_string());
    }
    if !semver_like(&pkg.integration.version) {
        errors.push(format!(
            "integration.toml: version '{}' is not valid SemVer",
            pkg.integration.version
        ));
    }
    const VALID_MATURITY: &[&str] = &["draft", "beta", "production"];
    if !VALID_MATURITY.contains(&pkg.integration.maturity.as_str()) {
        errors.push(format!(
            "integration.toml: maturity '{}' must be one of {VALID_MATURITY:?}",
            pkg.integration.maturity
        ));
    }

    errors
}

/// Joined capability check: every
/// `capabilities.toml` entry must have a matching `permissions.toml` entry
/// or it "is not a capability" and fails lint. Also checks
/// the provenance default: a non-read_only (write) capability's posture
/// must not be `auto` — write-capable capabilities are approval-required.
fn check_capability_policy_join(pkg: &LoadedIntegration) -> Vec<String> {
    use std::collections::BTreeMap;

    let mut errors = Vec::new();
    let permissions_by_cap: BTreeMap<&str, &PermissionEntry> = pkg
        .permissions
        .permission
        .iter()
        .map(|p| (p.capability.as_str(), p))
        .collect();

    for cap in &pkg.capabilities.capability {
        match permissions_by_cap.get(cap.id.as_str()) {
            None => {
                errors.push(format!(
                    "capability '{}' is declared in capabilities.toml but has no matching entry \
                     in permissions.toml — an entry with no permission policy is not a \
                     capability",
                    cap.id
                ));
            }
            Some(policy) => {
                if !cap.read_only && policy.approval_posture == POSTURE_AUTO {
                    errors.push(format!(
                        "capability '{}' is write-capable (read_only = false) but \
                         permissions.toml sets approval_posture = 'auto' — write-capable \
                         capabilities default to approval-required",
                        cap.id
                    ));
                }
            }
        }
    }

    let declared_caps: std::collections::BTreeSet<&str> = pkg
        .capabilities
        .capability
        .iter()
        .map(|c| c.id.as_str())
        .collect();
    for policy in &pkg.permissions.permission {
        if !declared_caps.contains(policy.capability.as_str()) {
            errors.push(format!(
                "permissions.toml declares a policy for '{}', which is not defined in \
                 capabilities.toml",
                policy.capability
            ));
        }
    }

    errors
}

fn integration_lint(path: &Path, json_output: bool) -> Result<()> {
    let pkg = load_integration(path)?;

    let mut errors = check_schema_shape(&pkg);
    errors.extend(check_capability_policy_join(&pkg));

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
        print_section_header("Integration Lint");
        if errors.is_empty() {
            print_status_line(
                &pkg.integration.integration_id,
                Status::Ok,
                "no drift, schema-valid",
            );
        } else {
            for error in &errors {
                print_status_line(&pkg.integration.integration_id, Status::Error, error);
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
// integration install
// ---------------------------------------------------------------------

pub(crate) fn integrations_dir(apxm_home: &Path) -> PathBuf {
    apxm_home.join("integrations")
}

fn integration_install(path: &Path, force: bool, json_output: bool) -> Result<()> {
    let workspace_root = std::env::var(apxm_core::constants::env::APXM_WORKSPACE_ROOT)
        .ok()
        .map(PathBuf::from);
    integration_install_to(
        path,
        &apxm_core::env::apxm_home(),
        workspace_root.as_deref(),
        force,
        json_output,
    )
}

/// Installs an integration package under an explicit `apxm_home` root, and
/// best-effort copies it to `<workspace_root>/integrations/<id>/` when a
/// workspace root is given — see the module doc comment's "Fan-out" section
/// for why this filesystem copy (not an HTTP push) is the correct fan-out
/// mechanism to auth-ms/apxm-os/Studio. Split out from [`integration_install`]
/// so tests can point at tempdirs instead of mutating the process-global
/// `APXM_HOME`/`APXM_WORKSPACE_ROOT` env vars (this crate denies
/// `unsafe_code`, which `std::env::set_var` requires in Rust 2024) — same
/// pattern as `agent.rs::agent_install_to` / `org.rs::org_install_to`.
pub(crate) fn integration_install_to(
    path: &Path,
    apxm_home: &Path,
    workspace_root: Option<&Path>,
    force: bool,
    json_output: bool,
) -> Result<()> {
    // Lint before install: an ungoverned (policy-missing) capability must
    // never reach a scanned catalog root.
    integration_lint(path, true)
        .context("refusing to install: package does not pass 'apxm integration lint'")?;

    let pkg = load_integration(path)?;
    let id = pkg.integration.integration_id.clone();
    let dest = integrations_dir(apxm_home).join(&id);

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
        .with_context(|| format!("Failed to create {}", integrations_dir(apxm_home).display()))?;
    copy_dir_recursive(path, &dest)?;

    // Best-effort fan-out copy: the shared workspace root every
    // non-server consumer (auth-ms, apxm-os, Studio) actually scans. Never
    // fails the install — a missing/unreachable shared workspace is normal
    // outside a co-located dev/CI stack.
    let workspace_copy_warning =
        workspace_root.and_then(|root| copy_to_workspace_root(path, &id, root, force));

    if let Some(warning) = &workspace_copy_warning {
        eprintln!("{warning}");
    }

    if json_output {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "id": id,
                "installed_to": dest.display().to_string(),
                "workspace_copy_warning": workspace_copy_warning,
            }))?
        );
    } else {
        print_section_header("Integration Installed");
        print_status_line(&id, Status::Ok, &dest.display().to_string());
    }
    Ok(())
}

/// Copies `path` to `<workspace_root>/integrations/<id>/`, matching
/// `studio/scripts/stack-lib.sh::sync_integrations`'s own precedent for
/// keeping every scanner's catalog in sync. Returns a human-readable warning
/// string on any failure; the caller treats this as best-effort and never
/// fails the install because of it.
fn copy_to_workspace_root(
    path: &Path,
    id: &str,
    workspace_root: &Path,
    force: bool,
) -> Option<String> {
    let dest = workspace_root.join("integrations").join(id);

    if dest.exists() {
        if !force {
            return Some(format!(
                "warning: could not copy to workspace-shared catalog at '{}' — destination \
                 already exists (pass --force to overwrite); auth-ms/apxm-os/Studio will keep \
                 seeing their previously-synced copy until this is resolved by hand",
                dest.display()
            ));
        }
        if let Err(err) = fs::remove_dir_all(&dest) {
            return Some(format!(
                "warning: could not copy to workspace-shared catalog at '{}': {err}",
                dest.display()
            ));
        }
    }
    if let Some(parent) = dest.parent()
        && let Err(err) = fs::create_dir_all(parent)
    {
        return Some(format!(
            "warning: could not create workspace-shared integrations root '{}': {err}",
            parent.display()
        ));
    }
    if let Err(err) = copy_dir_recursive(path, &dest) {
        return Some(format!(
            "warning: could not copy to workspace-shared catalog at '{}': {err}",
            dest.display()
        ));
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn scaffold(dir: &Path, id: &str) {
        integration_new(id, Some(dir.to_path_buf()), None, true).expect("scaffold ok");
    }

    #[test]
    fn new_scaffolds_schema_valid_six_file_tree_with_policy_defaults() {
        let tmp = tempdir().unwrap();
        let root = tmp.path().join("demo");
        scaffold(&root, "demo");

        for rel in REQUIRED_FILES {
            assert!(root.join(rel).is_file(), "missing scaffolded file: {rel}");
        }

        let integration: IntegrationToml = read_toml(&root.join("integration.toml")).unwrap();
        assert_eq!(integration.integration_id, "demo");
        assert_eq!(integration.maturity, "draft");
        assert_eq!(
            integration.files.permissions.as_deref(),
            Some("permissions.toml")
        );

        let capabilities: CapabilitiesToml = read_toml(&root.join("capabilities.toml")).unwrap();
        assert_eq!(capabilities.capability.len(), 2);
        let probe = capabilities
            .capability
            .iter()
            .find(|c| c.id == "demo.probe")
            .unwrap();
        assert!(probe.read_only);
        let send = capabilities
            .capability
            .iter()
            .find(|c| c.id == "demo.send")
            .unwrap();
        assert!(!send.read_only);

        let permissions: PermissionsToml = read_toml(&root.join("permissions.toml")).unwrap();
        let probe_policy = permissions
            .permission
            .iter()
            .find(|p| p.capability == "demo.probe")
            .unwrap();
        assert_eq!(
            probe_policy.approval_posture, "auto",
            "read_only capability defaults to auto"
        );
        let send_policy = permissions
            .permission
            .iter()
            .find(|p| p.capability == "demo.send")
            .unwrap();
        assert_ne!(
            send_policy.approval_posture, "auto",
            "write capability must default to approval-required, not auto"
        );

        integration_lint(&root, true).expect("scaffolded integration should lint clean");
    }

    #[test]
    fn new_refuses_nonempty_destination() {
        let tmp = tempdir().unwrap();
        let root = tmp.path().join("demo");
        scaffold(&root, "demo");
        let err = integration_new("demo", Some(root.clone()), None, true).unwrap_err();
        assert!(err.to_string().contains("already exists"));
    }

    #[test]
    fn lint_catches_capability_without_policy() {
        let tmp = tempdir().unwrap();
        let root = tmp.path().join("nopolicy");
        scaffold(&root, "nopolicy");

        // Drop the permissions.toml entry for the write capability — a
        // capability without a policy half is not a capability.
        fs::write(
            root.join("permissions.toml"),
            "[[permission]]\n\
             capability = \"nopolicy.probe\"\n\
             approval_posture = \"auto\"\n",
        )
        .unwrap();

        let err = integration_lint(&root, true).expect_err("missing policy must fail lint");
        assert!(err.to_string().contains("lint error"));
    }

    #[test]
    fn lint_catches_write_capability_without_approval_required_posture() {
        let tmp = tempdir().unwrap();
        let root = tmp.path().join("badposture");
        scaffold(&root, "badposture");

        // The write capability ('badposture.send') is auto-postured — a
        // policy violation of the  provenance default.
        fs::write(
            root.join("permissions.toml"),
            "[[permission]]\n\
             capability = \"badposture.probe\"\n\
             approval_posture = \"auto\"\n\n\
             [[permission]]\n\
             capability = \"badposture.send\"\n\
             approval_posture = \"auto\"\n",
        )
        .unwrap();

        let err = integration_lint(&root, true)
            .expect_err("write capability with auto posture must fail lint");
        assert!(
            err.to_string().contains("lint error"),
            "expected a lint error, got: {err}"
        );

        let pkg = load_integration(&root).unwrap();
        let errors = check_capability_policy_join(&pkg);
        assert!(
            errors
                .iter()
                .any(|e| e.contains("badposture.send") && e.contains("approval-required")),
            "expected the write/approval-posture violation message, got: {errors:?}"
        );
    }

    #[test]
    fn lint_catches_orphaned_permission_entry() {
        let tmp = tempdir().unwrap();
        let root = tmp.path().join("orphan");
        scaffold(&root, "orphan");

        let permissions_toml = fs::read_to_string(root.join("permissions.toml")).unwrap();
        fs::write(
            root.join("permissions.toml"),
            format!(
                "{permissions_toml}\n[[permission]]\ncapability = \"orphan.nowhere\"\napproval_posture = \"auto\"\n"
            ),
        )
        .unwrap();

        let err = integration_lint(&root, true).expect_err("orphaned policy must fail lint");
        assert!(err.to_string().contains("lint error"));
    }

    #[test]
    fn install_places_files_under_apxm_home_integrations() {
        let tmp = tempdir().unwrap();
        let root = tmp.path().join("installable");
        scaffold(&root, "installable");
        let fake_home = tempdir().unwrap();

        integration_install_to(&root, fake_home.path(), None, false, true).expect("install ok");

        let dest = fake_home.path().join("integrations").join("installable");
        assert!(dest.join("integration.toml").is_file());
        assert!(dest.join("permissions.toml").is_file());

        let real_home = dirs::home_dir().unwrap_or_default();
        assert!(!real_home.join(".apxm/integrations/installable").exists());
    }

    #[test]
    fn install_refuses_lint_dirty_package() {
        let tmp = tempdir().unwrap();
        let root = tmp.path().join("dirty");
        scaffold(&root, "dirty");
        fs::write(
            root.join("permissions.toml"),
            "[[permission]]\ncapability = \"dirty.probe\"\napproval_posture = \"auto\"\n",
        )
        .unwrap();
        let fake_home = tempdir().unwrap();

        let err = integration_install_to(&root, fake_home.path(), None, false, true)
            .expect_err("install of a lint-dirty package must fail");
        assert!(err.to_string().contains("apxm integration lint"));
        assert!(
            !integrations_dir(fake_home.path()).join("dirty").exists(),
            "a failed install must not leave a partial copy"
        );
    }

    #[test]
    fn install_refuses_existing_destination_without_force() {
        let tmp = tempdir().unwrap();
        let root = tmp.path().join("dupe");
        scaffold(&root, "dupe");
        let fake_home = tempdir().unwrap();

        integration_install_to(&root, fake_home.path(), None, false, true)
            .expect("first install ok");
        let second = integration_install_to(&root, fake_home.path(), None, false, true);
        let err = second.expect_err("second install without --force must fail");
        assert!(err.to_string().contains("already exists"));
    }

    #[test]
    fn install_copies_to_workspace_root_when_given() {
        let tmp = tempdir().unwrap();
        let root = tmp.path().join("shared_copy");
        scaffold(&root, "shared_copy");
        let fake_home = tempdir().unwrap();
        let fake_workspace = tempdir().unwrap();

        integration_install_to(
            &root,
            fake_home.path(),
            Some(fake_workspace.path()),
            false,
            true,
        )
        .expect("install ok");

        let dest = fake_home.path().join("integrations").join("shared_copy");
        assert!(
            dest.join("integration.toml").is_file(),
            "APXM_HOME copy must exist"
        );

        let shared_copy = fake_workspace
            .path()
            .join("integrations")
            .join("shared_copy")
            .join("integration.toml");
        assert!(shared_copy.is_file(), "workspace-shared copy must exist");
    }

    #[test]
    fn install_workspace_copy_failure_does_not_fail_install() {
        let tmp = tempdir().unwrap();
        let root = tmp.path().join("copy-fail");
        scaffold(&root, "copy-fail");
        let fake_home = tempdir().unwrap();

        // A workspace root that is actually a *file* (not a directory) makes
        // the workspace copy fail; the install itself must still succeed.
        let bogus_workspace_marker = tempdir().unwrap();
        let bogus_workspace = bogus_workspace_marker.path().join("not-a-dir");
        fs::write(&bogus_workspace, "not a directory").unwrap();

        integration_install_to(&root, fake_home.path(), Some(&bogus_workspace), false, true)
            .expect("install must succeed even when the best-effort workspace copy fails");

        let dest = fake_home.path().join("integrations").join("copy-fail");
        assert!(dest.join("integration.toml").is_file());
    }
}
