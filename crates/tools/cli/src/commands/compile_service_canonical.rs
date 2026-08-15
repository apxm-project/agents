//! Canonical package-to-AIR compile service command.

use std::env;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use apxm_core::constants::env as apxm_env;

use super::agent::{AgentToml, FrontendLanguage};

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
    super::agent::validate_agent_package(agent_dir)?;
    let (entry, frontend) = declared_agent_package_entry(agent_dir)?.ok_or_else(|| {
        anyhow::anyhow!(
            "{} must declare [compile].entry and [compile].frontend",
            agent_dir.join("agent.toml").display()
        )
    })?;
    super::agent::verify_agent_integrity(agent_dir)?;
    let air_json = match frontend {
        FrontendLanguage::Python => run_canonical_python_entry(&entry, config_path)?,
        FrontendLanguage::TypeScript => {
            run_canonical_typescript_entry(agent_dir, &entry, config_path)?
        }
    };
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
/// in program source, the `capabilities/<id>/handler.{py,ts}` handlers the package
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
        agent_dir
            .join("capabilities/<id>/handler.{py,ts}")
            .display(),
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
    canonical_entry_air(input, &output)
}

/// Bootstrap that turns a built module into AIR on stdout.
///
/// `[compile].entry` names a path, not an export, so the module is imported and
/// scanned for the single handle that can emit canonical AIR. Refusing an
/// ambiguous module keeps the declared entry the thing that decides what
/// compiles, rather than an export name the package could rename silently.
const TYPESCRIPT_AIR_BOOTSTRAP: &str = r#"
import { pathToFileURL } from "node:url";
const module = await import(pathToFileURL(process.argv[1]).href);
const handles = Object.entries(module).filter(
  ([, value]) => value && typeof value.canonicalAir === "function",
);
if (handles.length !== 1) {
  console.error(
    "expected exactly one Agent export exposing canonicalAir(), saw: [" +
      handles.map(([name]) => name).join(", ") +
      "]",
  );
  process.exit(1);
}
process.stdout.write(handles[0][1].canonicalAir());
"#;

/// Run a TypeScript package entry through its built JavaScript.
///
/// The authored `.ts` is not executable: the capture pass and the AIR bridge
/// are both JavaScript, so the package's own build output is what runs. The
/// build is deliberately not invoked here — every other gate in the tree builds
/// as its own step, and compiling should not mutate the package being compiled.
fn run_canonical_typescript_entry(
    agent_dir: &Path,
    entry: &Path,
    config_path: Option<&Path>,
) -> Result<String> {
    let built = built_typescript_entry(agent_dir, entry)?;

    let mut command = std::process::Command::new("node");
    // Run from the package root so the emitted source map names the authored
    // entry package-relative, matching what the package's own compile script
    // produces. The AIR is content-addressed, so a differing cwd is a differing
    // artifact.
    command
        .current_dir(agent_dir)
        .arg("--input-type=module")
        .arg("--eval")
        .arg(TYPESCRIPT_AIR_BOOTSTRAP)
        .arg(&built);
    if let Some(config_path) = config_path {
        command.env(apxm_env::APXM_CONFIG, config_path);
    }

    let output = match command.output() {
        Ok(output) => output,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            return Err(anyhow::anyhow!("Node interpreter not found on PATH"));
        }
        Err(err) => {
            return Err(anyhow::anyhow!(
                "Failed to run canonical TypeScript entry {} with node: {}",
                entry.display(),
                err
            ));
        }
    };
    canonical_entry_air(entry, &output)
}

/// Resolve the built JavaScript a TypeScript `[compile].entry` compiles to.
///
/// `tsc` places output under `outDir` keyed by the inferred or declared
/// `rootDir`, so a package that compiles the whole tree emits `dist/src/main.js`
/// while one rooted at `src/` emits `dist/main.js`. Both are probed rather than
/// assumed, and a miss names every path tried plus the build that produces them.
fn built_typescript_entry(agent_dir: &Path, entry: &Path) -> Result<PathBuf> {
    let relative = entry.strip_prefix(agent_dir).unwrap_or(entry);
    let package_rooted = Path::new("dist").join(relative.with_extension("js"));
    let below_first_component: PathBuf = relative.iter().skip(1).collect();
    let source_rooted = (!below_first_component.as_os_str().is_empty())
        .then(|| Path::new("dist").join(below_first_component.with_extension("js")));

    let candidates: Vec<PathBuf> = std::iter::once(package_rooted)
        .chain(source_rooted)
        .collect();
    for candidate in &candidates {
        if agent_dir.join(candidate).is_file() {
            return Ok(candidate.clone());
        }
    }
    anyhow::bail!(
        "{} declares a TypeScript [compile].entry {:?}, but no built module was found at [{}]. \
         Build the package first, for example with 'npm --prefix {} run build'",
        agent_dir.join("agent.toml").display(),
        relative.display(),
        candidates
            .iter()
            .map(|candidate| candidate.display().to_string())
            .collect::<Vec<_>>()
            .join(", "),
        agent_dir.display(),
    )
}

/// Shared stdout contract for every canonical entry: a successful process that
/// wrote non-empty UTF-8 AIR. Both frontends fail the same way.
fn canonical_entry_air(entry: &Path, output: &std::process::Output) -> Result<String> {
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(anyhow::anyhow!(
            "canonical entry {} failed: {}",
            entry.display(),
            stderr.trim()
        ));
    }
    let air = String::from_utf8(output.stdout.clone()).with_context(|| {
        format!(
            "canonical entry {} did not emit valid UTF-8",
            entry.display()
        )
    })?;
    let trimmed = air.trim();
    if trimmed.is_empty() {
        return Err(anyhow::anyhow!(
            "canonical entry {} produced no AIR output on stdout",
            entry.display()
        ));
    }
    Ok(trimmed.to_string())
}

fn declared_agent_package_entry(agent_dir: &Path) -> Result<Option<(PathBuf, FrontendLanguage)>> {
    let agent_path = agent_dir.join("agent.toml");
    let source: AgentToml = toml::from_str(
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
            let expected_extension = frontend.source_extension();
            if !entry.ends_with(expected_extension) {
                anyhow::bail!(
                    "{} declares [compile].frontend = {frontend}, but entry {entry:?} does not end in {expected_extension}",
                    agent_path.display(),
                );
            }
            Ok(Some((resolved, frontend)))
        }
    }
}
