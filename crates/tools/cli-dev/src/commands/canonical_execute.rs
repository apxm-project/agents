//! Execute canonical `apxm.air` through the Runtime Service.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use apxm_core::types::{HandlerLanguage, HandlerManifest};
use apxm_kernel::InvocationAdmission;
use apxm_program::air::AirModule;
use serde_json::{Value, json};

/// The exact package-handler implementation a composition root is supplied
/// with when it binds a package's own Capabilities.
///
/// The runtime never discovers a language worker. It is handed one per language
/// the manifest uses, together with the manifest those workers may evaluate,
/// exactly as it is handed every other implementation.
#[derive(Debug, Clone)]
pub struct AdmittedPackageHandlers {
    /// The private handler-worker the composition root selected for each
    /// language its manifest carries. A language with no entry here was not
    /// supplied, and a descriptor naming it is refused rather than dispatched.
    pub workers: BTreeMap<HandlerLanguage, PackageHandlerWorkerCommand>,
    /// The validated manifest those workers may evaluate, and nothing else.
    pub manifest: HandlerManifest,
    /// Host-issued read-only decisions; package metadata never supplies them.
    pub trusted_read_only: std::collections::BTreeSet<String>,
}

/// How one language's private worker is started.
#[derive(Debug, Clone)]
pub struct PackageHandlerWorkerCommand {
    /// The interpreter that runs the worker entry.
    pub interpreter: String,
    /// The private worker entry itself.
    pub entry: PathBuf,
}

/// Bind the package's shipped handlers only when the grant set and the
/// dispatchable set are the same set.
///
/// # Errors
///
/// Returns an error when the package fails integrity verification, when its
/// manifest is absent or non-conforming, when the manifest and the shipped
/// handler sources disagree, or when a private worker the manifest needs is not
/// installed.
pub fn admitted_package_handlers(root: &Path) -> Result<Option<AdmittedPackageHandlers>> {
    super::agent::verify_agent_integrity(root)?;
    let manifest = super::agent::load_tools_manifest(root)?;
    let described: BTreeMap<String, HandlerLanguage> = manifest
        .handlers
        .iter()
        .map(|entry| (entry.name.clone(), entry.language))
        .collect();
    let shipped = super::agent::shipped_capability_handlers(root)?;
    if described != shipped {
        let names = |handlers: &BTreeMap<String, HandlerLanguage>| {
            handlers
                .iter()
                .map(|(name, language)| format!("{name} ({language:?})"))
                .collect::<Vec<_>>()
                .join(", ")
        };
        anyhow::bail!(
            "agent package '{}' would grant [{}] but ships executable handlers for [{}]; run \
             'apxm build {}' so the ids the grant set claims are exactly the ids the \
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
    let mut workers = BTreeMap::new();
    for language in described.into_values() {
        if let std::collections::btree_map::Entry::Vacant(slot) = workers.entry(language) {
            let packaging = super::agent::agent_packaging(language);
            slot.insert(PackageHandlerWorkerCommand {
                interpreter: packaging.interpreter.to_string(),
                entry: super::agent::installed_agent_packaging_entry(
                    language,
                    handler_worker_entry(language),
                )?,
            });
        }
    }
    Ok(Some(AdmittedPackageHandlers {
        workers,
        manifest,
        trusted_read_only: std::collections::BTreeSet::new(),
    }))
}

fn handler_worker_entry(language: HandlerLanguage) -> &'static str {
    match language {
        HandlerLanguage::Python => "tool_worker.py",
        HandlerLanguage::TypeScript => "tool-worker.mjs",
    }
}

fn execute_via_runtime_service(
    artifact_bytes: &[u8],
    admission: InvocationAdmission,
    release_bytes: Vec<u8>,
    provenance_bytes: Vec<u8>,
    handlers: Option<AdmittedPackageHandlers>,
    package_root: Option<PathBuf>,
) -> Result<Value> {
    let mut service = apxm_runtime_service::RuntimeService::default();
    let handlers = handlers.map(|handlers| apxm_runtime_service::AdmittedPackageHandlers {
        workers: handlers
            .workers
            .into_iter()
            .map(|(language, command)| {
                (
                    language,
                    apxm_runtime_service::PackageHandlerWorkerCommand {
                        interpreter: command.interpreter,
                        entry: command.entry,
                    },
                )
            })
            .collect(),
        manifest: handlers.manifest,
        trusted_read_only: handlers.trusted_read_only,
    });
    service.bind_package(handlers, package_root);
    let digest = service.admit_artifact(artifact_bytes.to_vec());
    if digest != admission.artifact_digest {
        anyhow::bail!(
            "artifact digest {} does not match Invocation Admission {}",
            digest,
            admission.artifact_digest
        );
    }
    let created = service
        .handle(
            &apxm_runtime_protocol::RuntimeHandshake {
                protocol_version: apxm_runtime_protocol::RUNTIME_PROTOCOL_VERSION.to_owned(),
            },
            apxm_runtime_protocol::RuntimeRequest::ProgramInstanceCreate {
                request_id: "execute-canonical".to_owned(),
                artifact_digest: digest,
            },
        )
        .map_err(|error| anyhow::anyhow!("Runtime Service refused the artifact: {error:?}"))?;
    let apxm_runtime_protocol::RuntimeResult::ProgramInstanceCreated {
        program_instance_id,
        owner_claim,
        ..
    } = created
    else {
        anyhow::bail!("Runtime Service did not create a Program Instance");
    };
    service
        .bind_admission(
            &program_instance_id,
            apxm_runtime_service::InvocationMaterials {
                admission,
                release_bytes,
                provenance_bytes,
            },
        )
        .map_err(|error| anyhow::anyhow!(error))?;
    let started = service
        .handle(
            &apxm_runtime_protocol::RuntimeHandshake {
                protocol_version: apxm_runtime_protocol::RUNTIME_PROTOCOL_VERSION.to_owned(),
            },
            apxm_runtime_protocol::RuntimeRequest::ProgramInvocationStart {
                request_id: "execute-canonical".to_owned(),
                program_instance_id,
                owner_claim,
                input: json!({}),
            },
        )
        .map_err(|error| anyhow::anyhow!("Runtime Service refused the invocation: {error:?}"))?;
    match started {
        apxm_runtime_protocol::RuntimeResult::ProgramInvocationStarted { .. } => service
            .last_output()
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("Runtime Service produced no committed output")),
        apxm_runtime_protocol::RuntimeResult::Failed { code, .. } => {
            anyhow::bail!("Runtime Service invocation failed: {code}")
        }
        other => anyhow::bail!("Runtime Service returned {other:?}"),
    }
}

/// Admit fixture bytes and invoke them through the Runtime Service.
pub fn execute_canonical_command(
    input: PathBuf,
    invocation_admission: PathBuf,
    release: PathBuf,
    provenance: PathBuf,
    handlers: Option<AdmittedPackageHandlers>,
    package_root: Option<PathBuf>,
    json_output: bool,
) -> Result<()> {
    let (_air, artifact_bytes) = load_canonical_air(&input)?;
    let admission_bytes = read_exact_bytes(&invocation_admission, "Invocation Admission")?;
    let admission: InvocationAdmission =
        serde_json::from_slice(&admission_bytes).with_context(|| {
            format!(
                "{} must contain exact {} JSON",
                invocation_admission.display(),
                apxm_kernel::INVOCATION_ADMISSION_SCHEMA
            )
        })?;
    let release_bytes = read_exact_bytes(&release, "release")?;
    let provenance_bytes = read_exact_bytes(&provenance, "provenance")?;
    let output = execute_via_runtime_service(
        &artifact_bytes,
        admission,
        release_bytes,
        provenance_bytes,
        handlers,
        package_root,
    )?;
    if json_output {
        println!("{}", serde_json::to_string(&output)?);
    } else {
        println!("{}", serde_json::to_string_pretty(&output)?);
    }
    Ok(())
}

fn read_exact_bytes(path: &Path, label: &str) -> Result<Vec<u8>> {
    std::fs::read(path)
        .with_context(|| format!("failed to read exact {label} bytes from {}", path.display()))
}

fn load_canonical_air(input: &Path) -> Result<(AirModule, Vec<u8>)> {
    let bytes = read_exact_bytes(input, "canonical AIR")?;
    let text = std::str::from_utf8(&bytes)
        .with_context(|| format!("{} must contain UTF-8 canonical AIR JSON", input.display()))?;
    let air: AirModule = serde_json::from_str(text)
        .with_context(|| format!("{} must contain canonical apxm.air JSON", input.display()))?;
    let verdict = air.verify();
    if !verdict.is_accepted() {
        let diagnostics = verdict
            .into_diagnostics()
            .into_iter()
            .map(|diagnostic| {
                json!({
                    "code": diagnostic.code.slug(),
                    "location": diagnostic.location,
                    "message": diagnostic.message,
                })
            })
            .collect::<Vec<_>>();
        anyhow::bail!(
            "canonical AIR verification failed: {}",
            serde_json::to_string(&diagnostics)?
        );
    }
    Ok((air, bytes))
}

#[cfg(test)]
mod tests {
    /// The grant set and the dispatchable set are the same set, or the
    /// composition root refuses to bind the package at all.
    #[test]
    fn a_package_whose_manifest_lost_a_shipped_handler_is_refused() {
        use std::fs;

        use tempfile::tempdir;

        let tmp = tempdir().unwrap();
        let root = tmp.path().join("drifted");
        crate::commands::agent::agent_new(
            "drifted",
            Some(root.clone()),
            None,
            "looped-agent",
            true,
        )
        .expect("scaffold ok");
        fs::create_dir_all(root.join("capabilities/propose_edit")).unwrap();
        fs::write(
            root.join("capabilities/propose_edit/handler.ts"),
            "export function proposeEdit() {}\n",
        )
        .unwrap();
        assert!(
            crate::commands::agent::granted_capability_ids(&root)
                .unwrap()
                .contains("propose_edit")
        );
        crate::commands::agent::seal_agent_integrity_for_test(&root).unwrap();

        let error = super::admitted_package_handlers(&root).expect_err("the drift must be refused");
        let message = error.to_string();
        assert!(
            message.contains("propose_edit") && message.contains("apxm build"),
            "the refusal names the id the grant set claims and how to make it true: {message}"
        );
    }
}
