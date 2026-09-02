//! Execute canonical `apxm.air` through the Runtime Service.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use apxm_core::types::host_capability::{HostCapabilityOutcomeKind, is_host_capability_ref};
use apxm_core::types::{HandlerLanguage, HandlerManifest};
use apxm_kernel::InvocationAdmission;
use apxm_program::{ExecutableArtifact, air::AirModule};
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
    air: &AirModule,
    artifact_bytes: &[u8],
    admission: InvocationAdmission,
    release_bytes: Vec<u8>,
    provenance_bytes: Vec<u8>,
    handlers: Option<AdmittedPackageHandlers>,
    package_root: Option<PathBuf>,
) -> Result<Value> {
    // A program that invokes a host-fulfilled Capability parks rather than
    // running it: APXM publishes a request and waits for the embedding host
    // (ADR-0025). This command *is* that host for the fixture, so it drives the
    // resumable path and answers each request as it is published.
    let host_fulfilled = air
        .invoked_capability_refs()
        .into_iter()
        .any(is_host_capability_ref);
    let mut service = apxm_runtime_service::RuntimeService::in_memory()
        .with_embedded_read_access()
        .with_output_access_scope_ref("scope.execute-canonical".to_owned());
    if !host_fulfilled {
        service = service.with_single_shot_invocations();
    }
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
                owner_claim: owner_claim.clone(),
                input: json!({}),
            },
        )
        .map_err(|error| anyhow::anyhow!("Runtime Service refused the invocation: {error:?}"))?;
    match started {
        apxm_runtime_protocol::RuntimeResult::ProgramInvocationStarted {
            program_invocation_id,
            ..
        } => {
            let invocation = apxm_runtime_protocol::ProgramInvocationId::new(program_invocation_id)
                .map_err(|error| {
                    anyhow::anyhow!("Runtime Service returned an invalid invocation: {error}")
                })?;
            if host_fulfilled {
                settle_host_capability_requests(&mut service, &invocation, &owner_claim)?;
            }
            let context = |purpose| apxm_runtime_protocol::ReadContext {
                request_id: apxm_runtime_protocol::RequestId::new("execute-canonical-read")
                    .expect("static request id"),
                scope_ref: apxm_runtime_protocol::ScopeRef::new("scope.execute-canonical")
                    .expect("static scope"),
                principal_ref: apxm_runtime_protocol::PrincipalRef::new(
                    "principal.execute-canonical",
                )
                .expect("static principal"),
                grant_ref: apxm_runtime_protocol::GrantRef::new("grant.execute-canonical")
                    .expect("static grant"),
                correlation_id: None,
                purpose,
            };
            let inspection = service
                .handle_v2(
                    &apxm_runtime_protocol::RuntimeHandshakeV2::server(),
                    apxm_runtime_protocol::RuntimeRequestV2::ProgramInvocationInspect {
                        context: context(apxm_runtime_protocol::ReadPurpose::Inspection),
                        program_invocation_id: invocation.clone(),
                        node_execution_id: None,
                    },
                )
                .map_err(|error| {
                    anyhow::anyhow!("failed to inspect committed invocation: {error:?}")
                })?;
            let apxm_runtime_protocol::RuntimeResultV2::ProgramInvocationInspection {
                inspection,
                ..
            } = inspection
            else {
                anyhow::bail!("Runtime Service returned no committed invocation inspection");
            };
            let output_ref =
                inspection.output_refs.first().cloned().ok_or_else(|| {
                    anyhow::anyhow!("Runtime Service committed no output reference")
                })?;
            let content_ref = apxm_runtime_protocol::ContentRef::new(output_ref.as_str())
                .map_err(|error| anyhow::anyhow!("invalid committed output reference: {error}"))?;
            let content = service
                .handle_v2(
                    &apxm_runtime_protocol::RuntimeHandshakeV2::server(),
                    apxm_runtime_protocol::RuntimeRequestV2::ContentRead {
                        context: context(apxm_runtime_protocol::ReadPurpose::Content),
                        content_ref,
                    },
                )
                .map_err(|error| anyhow::anyhow!("failed to read committed output: {error:?}"))?;
            let apxm_runtime_protocol::RuntimeResultV2::Content { content, .. } = content else {
                anyhow::bail!("Runtime Service returned no committed output content");
            };
            serde_json::from_slice(&content.bytes)
                .map_err(|error| anyhow::anyhow!("committed output is not JSON: {error}"))
        }
        apxm_runtime_protocol::RuntimeResult::Failed { code, .. } => {
            anyhow::bail!("Runtime Service invocation failed: {code}")
        }
        other => anyhow::bail!("Runtime Service returned {other:?}"),
    }
}

/// Answer every host-fulfilled Capability request this invocation publishes.
///
/// The fixture's host is this command. It reads the published requests off the
/// observation stream — the same surface an embedding control plane reads —
/// and settles each with `capability_fulfill`, handing back the request's own
/// arguments as the output so the settlement is checkable without inventing a
/// system behind it. It stops when the invocation publishes no request it has
/// not already answered.
fn settle_host_capability_requests(
    service: &mut apxm_runtime_service::RuntimeService,
    invocation: &apxm_runtime_protocol::ProgramInvocationId,
    owner_claim: &apxm_runtime_protocol::RuntimeOwnerClaim,
) -> Result<()> {
    let mut settled = BTreeSet::new();
    loop {
        let page = service
            .handle_v2(
                &apxm_runtime_protocol::RuntimeHandshakeV2::server(),
                apxm_runtime_protocol::RuntimeRequestV2::ObservationSubscribe {
                    context: apxm_runtime_protocol::ReadContext {
                        request_id: apxm_runtime_protocol::RequestId::new(
                            "execute-canonical-host",
                        )
                        .expect("static request id"),
                        scope_ref: apxm_runtime_protocol::ScopeRef::new("scope.execute-canonical")
                            .expect("static scope"),
                        principal_ref: apxm_runtime_protocol::PrincipalRef::new(
                            "principal.execute-canonical",
                        )
                        .expect("static principal"),
                        grant_ref: apxm_runtime_protocol::GrantRef::new("grant.execute-canonical")
                            .expect("static grant"),
                        correlation_id: None,
                        purpose: apxm_runtime_protocol::ReadPurpose::Observation,
                    },
                    program_invocation_id: invocation.clone(),
                    after_cursor: None,
                    limit: 1000,
                },
            )
            .map_err(|error| anyhow::anyhow!("failed to read the observation stream: {error:?}"))?;
        let apxm_runtime_protocol::RuntimeResultV2::ObservationPage { page, .. } = page else {
            anyhow::bail!("Runtime Service returned no observation page");
        };
        let Some(request) = page
            .items
            .iter()
            .filter_map(|observation| {
                (observation.observation_kind
                    == apxm_runtime_protocol::ObservationKind::CapabilityRequested)
                    .then(|| observation.host_capability.as_ref())
                    .flatten()
            })
            .find(|request| !settled.contains(&request.capability_request_id))
            .cloned()
        else {
            return Ok(());
        };
        settled.insert(request.capability_request_id.clone());
        let settlement = service
            .handle(
                &apxm_runtime_protocol::RuntimeHandshake {
                    protocol_version: apxm_runtime_protocol::RUNTIME_PROTOCOL_VERSION.to_owned(),
                },
                apxm_runtime_protocol::RuntimeRequest::CapabilityFulfill {
                    request_id: format!("execute-canonical.{}", settled.len()),
                    owner_claim: owner_claim.clone(),
                    capability_request_id: request.capability_request_id.clone(),
                    outcome: HostCapabilityOutcomeKind::Ok,
                    output: Some(request.input.clone().unwrap_or_else(|| "{}".to_owned())),
                    receipt_ref: Some("receipt.execute-canonical".to_owned()),
                    message: None,
                },
            )
            .map_err(|error| {
                anyhow::anyhow!("Runtime Service refused the settlement: {error:?}")
            })?;
        match settlement {
            apxm_runtime_protocol::RuntimeResult::CapabilitySettled { .. } => {}
            apxm_runtime_protocol::RuntimeResult::Failed { code, .. } => {
                anyhow::bail!(
                    "Runtime Service refused to settle {}: {code}",
                    request.capability_ref
                )
            }
            other => anyhow::bail!("Runtime Service returned {other:?}"),
        }
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
        &_air,
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
    let artifact = ExecutableArtifact::from_air(&air).map_err(|error| {
        anyhow::anyhow!("failed to seal canonical executable artifact: {error}")
    })?;
    let artifact_bytes = artifact.encode().map_err(|error| {
        anyhow::anyhow!("failed to encode canonical executable artifact: {error}")
    })?;
    Ok((air, artifact_bytes))
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
