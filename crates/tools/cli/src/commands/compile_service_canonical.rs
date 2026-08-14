//! Canonical package-to-AIR compile service command.

use std::env;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use apxm_core::constants::env as apxm_env;
use apxm_core::types::ApxmPathFormat;
use serde::Deserialize;

use super::agent::{CompileToml, FrontendLanguage};

/// Compile a canonical-authored agent package to canonical `apxm.air` JSON.
pub fn compile_service_canonical_command(
    agent_dir: PathBuf,
    config: Option<PathBuf>,
) -> Result<()> {
    let air_json = emit_canonical_air_from_agent(&agent_dir, config.as_deref())?;
    use std::io::Write;
    let stdout = std::io::stdout();
    let mut handle = stdout.lock();
    handle.write_all(air_json.as_bytes())?;
    Ok(())
}

pub(crate) fn emit_canonical_air_from_agent(
    agent_dir: &Path,
    config_path: Option<&Path>,
) -> Result<String> {
    if !agent_dir.is_dir() {
        return Err(anyhow::anyhow!(
            "'{}' is not a directory",
            agent_dir.display()
        ));
    }
    let entry = declared_agent_package_entry(agent_dir)?.ok_or_else(|| {
        anyhow::anyhow!(
            "{} must declare [compile].entry and [compile].frontend",
            agent_dir.join("agent.toml").display()
        )
    })?;
    super::agent::verify_agent_integrity(agent_dir)?;
    if !ApxmPathFormat::from_path(&entry).is_python_frontend() {
        return Err(anyhow::anyhow!(
            "canonical compile-service currently supports a Python [compile].entry authored on the apxm_program frontend"
        ));
    }
    let air_json = run_canonical_python_entry(&entry, config_path)?;
    let module: apxm_program::air::AirModule =
        serde_json::from_str(&air_json).with_context(|| {
            format!(
                "canonical entry {} did not emit valid apxm.air AIR JSON on stdout",
                entry.display()
            )
        })?;
    check_capability_references_are_granted(agent_dir, &module)?;
    check_package_permissions_only_tighten(agent_dir, &module)?;
    Ok(air_json)
}

/// Resolve the compiled program's authored permission requests against the
/// package's own `agent.toml [permissions]`, and refuse to emit AIR the stack
/// rejects.
///
/// This is the one place in the tree where both halves of the shipped
/// resolution stack are simultaneously known: the program has just been lowered,
/// so `AirModule::capability_permission_requests` carries what its source
/// requested, and the package that ships it is right here on disk. Every other
/// producer knows only one of the two — `agent lint` reads a package without
/// compiling it, and `execute-canonical` is handed AIR without a package — so
/// until a caller held both, `agent.toml` could hand back authority the program
/// had itself declined and nothing would notice. A widening is not a warning
/// and not something the artifact records: the compile does not produce AIR at
/// all.
fn check_package_permissions_only_tighten(
    agent_dir: &Path,
    module: &apxm_program::air::AirModule,
) -> Result<()> {
    super::agent::resolve_package_permission_layers(
        agent_dir,
        &module.capability_permission_requests,
    )
    .map(|_| ())
}

/// Hold every Capability the compiled program names against what the package
/// can actually supply.
///
/// This is the join the namespaces never made: the reference an author writes
/// in program source, the `capabilities/<id>/handler.ts` handlers the package
/// ships, and the runtime's builtin allowlist. Without it a program could name
/// a Capability that is in none of them and still compile, produce an artifact,
/// and run — the reference simply resolved to nothing at the registry, far past
/// the point where the author could see the mistake.
fn check_capability_references_are_granted(
    agent_dir: &Path,
    module: &apxm_program::air::AirModule,
) -> Result<()> {
    let granted = super::agent::granted_capability_ids(agent_dir)?;
    let mut ungranted: Vec<&str> = module
        .semantic_operations
        .iter()
        .filter(|operation| operation.op == apxm_program::SemanticOpKind::CapabilityInvoke)
        .filter_map(|operation| {
            operation
                .operands
                .iter()
                .find(|operand| operand.slot == apxm_ais::SLOT_CAPABILITY_REF)
                .map(|operand| operand.value_id.as_str())
        })
        .filter(|capability_ref| !granted.contains(*capability_ref))
        .collect();
    ungranted.sort_unstable();
    ungranted.dedup();
    if ungranted.is_empty() {
        return Ok(());
    }
    anyhow::bail!(
        "{} authors {} no implementation can satisfy: [{}]. A Capability reference must name a \
         built-in id or an id this package ships a handler for at {}",
        agent_dir.join("agent.toml").display(),
        if ungranted.len() == 1 {
            "a Capability"
        } else {
            "Capabilities"
        },
        ungranted.join(", "),
        agent_dir.join("capabilities/<id>/handler.ts").display(),
    )
}

fn run_canonical_python_entry(input: &Path, config_path: Option<&Path>) -> Result<String> {
    let mut pythonpath_entries = Vec::new();
    if let Some(parent) = input.parent() {
        pythonpath_entries.push(parent.to_path_buf());
    }
    if let Some(existing) = env::var_os(apxm_env::PYTHONPATH) {
        pythonpath_entries.extend(env::split_paths(&existing));
    }
    let pythonpath = (!pythonpath_entries.is_empty())
        .then(|| env::join_paths(pythonpath_entries))
        .transpose()
        .context("Failed to build PYTHONPATH for the canonical APXM Python frontend")?;

    let mut output = None;
    for candidate in ["python3", "python"] {
        let mut command = std::process::Command::new(candidate);
        // Canonical package entries may expose both their frontend graph and
        // AIR for parity tooling. The compile service must request the AIR
        // representation explicitly so a package-level entry cannot
        // accidentally return a graph that this command then misclassifies.
        command.arg(input).arg("--air");
        if let Some(pythonpath) = pythonpath.as_ref() {
            command.env(apxm_env::PYTHONPATH, pythonpath);
        }
        if let Some(config_path) = config_path {
            command.env(apxm_env::APXM_CONFIG, config_path);
        }
        match command.output() {
            Ok(result) => {
                output = Some(result);
                break;
            }
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => continue,
            Err(err) => {
                return Err(anyhow::anyhow!(
                    "Failed to run canonical Python entry {} with {}: {}",
                    input.display(),
                    candidate,
                    err
                ));
            }
        }
    }
    let output = output.ok_or_else(|| {
        anyhow::anyhow!("Python interpreter not found on PATH (tried python3, python)")
    })?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(anyhow::anyhow!(
            "canonical Python entry {} failed: {}",
            input.display(),
            stderr.trim()
        ));
    }
    let air = String::from_utf8(output.stdout).with_context(|| {
        format!(
            "canonical entry {} did not emit valid UTF-8",
            input.display()
        )
    })?;
    let trimmed = air.trim();
    if trimmed.is_empty() {
        return Err(anyhow::anyhow!(
            "canonical entry {} produced no AIR output on stdout",
            input.display()
        ));
    }
    Ok(trimmed.to_string())
}

#[derive(Debug, Deserialize, Default)]
struct AgentPackageToml {
    #[serde(default)]
    compile: Option<CompileToml>,
}

fn declared_agent_package_entry(agent_dir: &Path) -> Result<Option<PathBuf>> {
    let agent_path = agent_dir.join("agent.toml");
    let source: AgentPackageToml = toml::from_str(
        &std::fs::read_to_string(&agent_path)
            .with_context(|| format!("Failed to read {}", agent_path.display()))?,
    )
    .with_context(|| format!("Failed to parse {}", agent_path.display()))?;
    let Some(compile) = source.compile else {
        return Ok(None);
    };

    match (compile.entry, compile.frontend) {
        (None, None) => Ok(None),
        (Some(_), None) => anyhow::bail!(
            "{} declares [compile].entry without [compile].frontend",
            agent_path.display()
        ),
        (None, Some(_)) => anyhow::bail!(
            "{} declares [compile].frontend without [compile].entry; remove [compile] for an entry-less declarative package",
            agent_path.display()
        ),
        (Some(entry), Some(frontend)) => {
            let entry_path = Path::new(&entry);
            if entry_path.is_absolute()
                || entry_path
                    .components()
                    .any(|component| matches!(component, std::path::Component::ParentDir))
            {
                anyhow::bail!(
                    "{} has an invalid [compile].entry {:?}; entries must be workspace-relative package paths",
                    agent_path.display(),
                    entry
                );
            }
            let resolved = agent_dir.join(entry_path);
            if !resolved.is_file() {
                anyhow::bail!(
                    "{} declares [compile].entry {:?}, but {} does not exist",
                    agent_path.display(),
                    entry,
                    resolved.display()
                );
            }
            if frontend != FrontendLanguage::Python {
                anyhow::bail!(
                    "canonical compile-service currently supports frontend = \"python\"; {} declares {frontend}",
                    agent_path.display()
                );
            }
            if !ApxmPathFormat::from_path(&resolved).is_python_frontend() {
                anyhow::bail!(
                    "{} declares [compile].frontend = {frontend}, but entry {entry:?} has the wrong source extension",
                    agent_path.display(),
                );
            }
            Ok(Some(resolved))
        }
    }
}
