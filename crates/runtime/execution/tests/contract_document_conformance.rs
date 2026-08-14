//! Conformance: the checked-in vectors are held against the closed Rust
//! carriers that actually implement each Port Contract's documents.
//!
//! `crates/machine/program/tests/contract_vector_conformance.rs` holds the same
//! vectors against their published JSON Schemas. That catches structural drift,
//! but a schema cannot express "this digest reproduces its own preimage" or
//! "this field agrees with that one", and ten of the published rejecting vectors
//! are exactly those. This file is where those verdicts are decided, by the code
//! that decides them in production.
//!
//! This crate is the one that sees every carrier at once — `apxm-execution`
//! depends on `apxm-program`, `apxm-inference`, and `apxm-kernel` — so the
//! runners live together instead of being scattered one per crate.
//!
//! Running these vectors for the first time found the carriers and the published
//! documents already disagreeing in seven places. Every one of them is recorded
//! in `RECORDED_DIVERGENCES` with what the carrier does and why, and the set is
//! asserted exactly: closing a gap fails this file and forces the entry out,
//! opening a new one fails it too. That is the point — an unread contract cannot
//! disagree with anything, so it rots in silence. These now disagree out loud.

use std::collections::BTreeSet;
use std::path::PathBuf;

use serde_json::Value;

// ---------------------------------------------------------------------------
// Vector loading
// ---------------------------------------------------------------------------

struct Vector {
    name: String,
    input: Value,
    expected_valid: bool,
}

fn agents_root() -> PathBuf {
    // crates/runtime/execution -> agents repo root
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../..")
}

fn load_json(path: PathBuf) -> Value {
    let text =
        std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
    serde_json::from_str(&text).unwrap_or_else(|e| panic!("parse {}: {e}", path.display()))
}

fn load_vectors(file: &str) -> Vec<Vector> {
    let doc = load_json(agents_root().join("contracts/vectors").join(file));
    let vectors: Vec<Vector> = doc
        .as_array()
        .unwrap_or_else(|| panic!("{file} is a JSON array"))
        .iter()
        .map(|entry| Vector {
            name: entry["name"].as_str().expect("vector name").to_string(),
            input: entry["input"].clone(),
            expected_valid: entry["expected_valid"].as_bool().expect("expected_valid"),
        })
        .collect();
    assert!(!vectors.is_empty(), "{file}: no vectors");
    vectors
}

fn load_schema(file: &str) -> Value {
    load_json(agents_root().join("contracts/schemas").join(file))
}

/// Decode into a closed carrier. Every carrier here is `deny_unknown_fields`, so
/// a document carrying a field the contract does not describe is refused before
/// any semantic check runs.
fn decodes<T: serde::de::DeserializeOwned>(document: &Value) -> Result<T, String> {
    serde_json::from_value::<T>(document.clone()).map_err(|e| e.to_string())
}

fn decode_only<T: serde::de::DeserializeOwned>(document: &Value) -> Result<(), String> {
    decodes::<T>(document).map(|_| ())
}

// ---------------------------------------------------------------------------
// The carriers
// ---------------------------------------------------------------------------

/// The exact admission path production takes for one published document, keyed
/// by the vector file that exercises it. A document with no entry here has no
/// Rust carrier at all and is enforced only by its published schema.
fn carrier_admits(file: &str, document: &Value) -> Result<(), String> {
    match file {
        "apxm.capability-invocation.json" => {
            let request = decodes::<apxm_program::CapabilityRequest>(document)?;
            request.validate().map_err(|e| format!("{e:?}"))
        }
        "apxm.capability-outcome.json" => decode_only::<apxm_program::CapabilityOutcome>(document),
        "apxm.durable-event-request.json" => decode_only::<apxm_kernel::EventAwait>(document),
        "apxm.durable-event-outcome.json" => decode_only::<apxm_kernel::EventOutcome>(document),
        "apxm.external-agent-request.json" => {
            decode_only::<apxm_kernel::AcpPromptRequest>(document)
        }
        "apxm.external-agent-outcome.json" => {
            decode_only::<apxm_kernel::AcpPromptOutcome>(document)
        }
        "apxm.program-composition-request.json" => {
            decode_only::<apxm_kernel::CompositionRequest>(document)
        }
        "apxm.program-composition-outcome.json" => {
            decode_only::<apxm_kernel::CompositionOutcome>(document)
        }
        "apxm.model-inference-request.json" => {
            decode_only::<apxm_inference::ModelCallRequest>(document)
        }
        "apxm.model-inference-outcome.json" => {
            decode_only::<apxm_inference::ModelOutcome>(document)
        }
        "apxm.model-binding.json" => decode_only::<apxm_inference::ResolvedModelBinding>(document),
        "apxm.execution-admission.json" => apxm_kernel::parse_execution_admission(document)
            .map(|_| ())
            .map_err(|e| format!("{e:?}")),
        "apxm.invocation-admission.json" => {
            let admission = decodes::<apxm_kernel::InvocationAdmission>(document)?;
            admission.validate().map_err(|e| format!("{e:?}"))
        }
        "apxm.inference-credential-lease.json" => {
            let identity = decodes::<apxm_inference::InferenceCredentialLeaseIdentity>(document)?;
            identity.validate().map_err(|e| format!("{e:?}"))
        }
        "apxm.inference-usage-lineage.json" => {
            let lineage = decodes::<apxm_inference::InferenceUsageLineage>(document)?;
            lineage.validate().map_err(|e| format!("{e:?}"))
        }
        "apxm.diagnostic-correlation.json" => {
            let correlation = decodes::<apxm_inference::DiagnosticCorrelation>(document)?;
            correlation.validate().map_err(|e| format!("{e:?}"))
        }
        "apxm.committed-native-model-usage.json" => {
            use apxm_execution::CommittedNativeModelUsage;

            let usage = decodes::<CommittedNativeModelUsage>(document)?;
            if usage.source_contract_digest != CommittedNativeModelUsage::SOURCE_CONTRACT_DIGEST {
                return Err(format!(
                    "source_contract_digest {} is not the digest of the schema this \
                     crate was built against",
                    usage.source_contract_digest
                ));
            }
            let derived =
                CommittedNativeModelUsage::measurement_id(&usage.commit_id, &usage.attempt.fact_id);
            if usage.usage_measurement_id != derived {
                return Err(format!(
                    "usage_measurement_id {} is not the identity {derived} derives \
                     from its own commit and attempt",
                    usage.usage_measurement_id
                ));
            }
            Ok(())
        }
        _ => panic!("{file} has no carrier entry"),
    }
}

/// Every vector file with a Rust carrier, in the order the families appear in
/// `contracts/port-contracts/`.
const FILES_WITH_A_CARRIER: &[&str] = &[
    "apxm.capability-invocation.json",
    "apxm.capability-outcome.json",
    "apxm.committed-native-model-usage.json",
    "apxm.diagnostic-correlation.json",
    "apxm.durable-event-outcome.json",
    "apxm.durable-event-request.json",
    "apxm.execution-admission.json",
    "apxm.external-agent-outcome.json",
    "apxm.external-agent-request.json",
    "apxm.inference-credential-lease.json",
    "apxm.inference-usage-lineage.json",
    "apxm.invocation-admission.json",
    "apxm.model-binding.json",
    "apxm.model-inference-outcome.json",
    "apxm.model-inference-request.json",
    "apxm.program-composition-outcome.json",
    "apxm.program-composition-request.json",
];

/// One place where a published vector and the carrier that implements its
/// contract already disagreed when the first reader was pointed at them.
struct Divergence {
    file: &'static str,
    vector: &'static str,
    /// What the carrier does, which is the opposite of what the vector claims.
    carrier_admits: bool,
    reason: &'static str,
}

/// The thirteen disagreements the first reader found, in three kinds.
///
/// **Stale content addresses.** Eight published vectors carry a digest that does
/// not reproduce under the verifier that owns its preimage. These vectors were
/// authored against an earlier preimage, the preimage moved, and no reader
/// existed to notice — the exact failure mode an unread contract has. Resolving
/// them means deciding which side is authoritative and republishing the other;
/// that is an owner's call on contract data, not a test's.
///
/// **Carrier and document describe different field sets.** Three published
/// documents name fields their carrier does not carry, or omit fields it
/// requires.
///
/// **Carrier looser than the contract.** Two published constraints are stated in
/// the schema and enforced nowhere at this entry point.
const RECORDED_DIVERGENCES: &[Divergence] = &[
    // Stale content addresses.
    Divergence {
        file: "apxm.inference-credential-lease.json",
        vector: "valid-target-bound-lease-identity",
        carrier_admits: false,
        reason: "the published lease_digest does not reproduce under \
                 InferenceCredentialLeaseIdentity's own digest preimage",
    },
    Divergence {
        file: "apxm.inference-credential-lease.json",
        vector: "valid-expiry-boundary-identity",
        carrier_admits: false,
        reason: "the published lease_digest does not reproduce under \
                 InferenceCredentialLeaseIdentity's own digest preimage",
    },
    Divergence {
        file: "apxm.inference-usage-lineage.json",
        vector: "valid-sealed-native-usage-lineage",
        carrier_admits: false,
        reason: "the published lineage_id does not reproduce under \
                 InferenceUsageLineage's own digest preimage",
    },
    Divergence {
        file: "apxm.inference-usage-lineage.json",
        vector: "valid-typed-failure-lineage",
        carrier_admits: false,
        reason: "the published lineage_id does not reproduce under \
                 InferenceUsageLineage's own digest preimage",
    },
    Divergence {
        file: "apxm.inference-usage-lineage.json",
        vector: "valid-bound-to-committed-evidence",
        carrier_admits: false,
        reason: "the published target commitment digests do not reproduce under \
                 InferenceTargetCommitment::commit",
    },
    Divergence {
        file: "apxm.diagnostic-correlation.json",
        vector: "valid-diagnostic-only-correlation",
        carrier_admits: false,
        reason: "the published correlation_digest does not reproduce under \
                 DiagnosticCorrelation's own digest preimage",
    },
    Divergence {
        file: "apxm.diagnostic-correlation.json",
        vector: "valid-disagreement-preserves-claimed-usage",
        carrier_admits: false,
        reason: "the published correlation_digest does not reproduce under \
                 DiagnosticCorrelation's own digest preimage",
    },
    Divergence {
        file: "apxm.diagnostic-correlation.json",
        vector: "valid-committed-target-diagnostic",
        carrier_admits: false,
        reason: "the published target commitment digests do not reproduce under \
                 InferenceTargetCommitment::commit",
    },
    // Carrier and document describe different field sets.
    Divergence {
        file: "apxm.committed-native-model-usage.json",
        vector: "valid-committed-native-model-usage",
        carrier_admits: false,
        reason: "the published attempt record carries fact_kind, which \
                 ModelAttemptRecordedFact does not",
    },
    Divergence {
        file: "apxm.model-binding.json",
        vector: "valid_resolved_binding",
        carrier_admits: false,
        reason: "apxm.model-binding publishes a standalone joined proof that no \
                 carrier emits; the Rust ResolvedModelBinding is a different \
                 document with the same title, and the admission-side form is \
                 AdmittedModelTarget — see the field accounting below",
    },
    Divergence {
        file: "apxm.model-inference-request.json",
        vector: "valid-exact-model-inference-request",
        carrier_admits: false,
        reason: "ModelCallRequest requires context_digest and authored_request, \
                 which the published request schema neither describes nor admits",
    },
    // Carrier looser than the contract.
    Divergence {
        file: "apxm.execution-admission.json",
        vector: "rejects_unconfined_type",
        carrier_admits: true,
        reason: "parse_execution_admission decodes only; the closed confinement \
                 vocabulary and the non-empty port-binding list are enforced by \
                 verify_execution_admission, which needs an issuer keyring, a \
                 nonce ledger, and a clock the vector does not carry",
    },
    Divergence {
        file: "apxm.external-agent-request.json",
        vector: "reject-empty-prompt",
        carrier_admits: true,
        reason: "AcpPromptRequest carries prompt as a bare String and does not \
                 enforce the published minLength of 1",
    },
];

/// Every vector's verdict is the verdict its carrier actually reaches, except
/// where a divergence is recorded above.
#[test]
fn every_vector_with_a_carrier_matches_that_carrier() {
    let recorded: BTreeSet<(&str, &str)> = RECORDED_DIVERGENCES
        .iter()
        .map(|divergence| (divergence.file, divergence.vector))
        .collect();
    assert_eq!(
        recorded.len(),
        RECORDED_DIVERGENCES.len(),
        "a divergence is recorded twice"
    );

    let mut seen: BTreeSet<(&str, &str)> = BTreeSet::new();
    let mut unrecorded: Vec<String> = Vec::new();

    for file in FILES_WITH_A_CARRIER {
        for Vector {
            name,
            input,
            expected_valid,
        } in load_vectors(file)
        {
            let verdict = carrier_admits(file, &input);
            if verdict.is_ok() == expected_valid {
                assert!(
                    !recorded.contains(&(*file, name.as_str())),
                    "{file}: vector '{name}' is recorded as diverging but the carrier \
                     now agrees with it; drop the entry"
                );
                continue;
            }
            let Some(divergence) = RECORDED_DIVERGENCES
                .iter()
                .find(|d| d.file == *file && d.vector == name)
            else {
                unrecorded.push(format!(
                    "  {file} :: {name} — vector says valid={expected_valid}, carrier \
                     says {:?}",
                    verdict.as_ref().err()
                ));
                continue;
            };
            assert_eq!(
                verdict.is_ok(),
                divergence.carrier_admits,
                "{file}: vector '{name}' diverges differently than recorded ({}): {verdict:?}",
                divergence.reason
            );
            seen.insert((divergence.file, divergence.vector));
        }
    }

    assert!(
        unrecorded.is_empty(),
        "these published vectors disagree with the carrier that implements their \
         contract, and the disagreement is not recorded:\n{}",
        unrecorded.join("\n")
    );
    assert_eq!(
        seen, recorded,
        "a recorded divergence no longer reproduces; the gap closed, so remove its entry"
    );
}

/// Every port slot the admission verifier can bind is a port this repository
/// publishes a Port Contract for. A slot whose contract is unpublished would be
/// admitted against nothing.
#[test]
fn every_admissible_port_slot_names_a_published_port_contract() {
    for schema_id in [
        apxm_kernel::CAPABILITY_PORT_SCHEMA,
        apxm_kernel::DURABLE_EVENT_PORT_SCHEMA,
        apxm_kernel::EXECUTION_COMMIT_PORT_SCHEMA,
        apxm_kernel::EXTERNAL_AGENT_PORT_SCHEMA,
        apxm_kernel::MODEL_INFERENCE_PORT_SCHEMA,
        apxm_kernel::PROGRAM_COMPOSITION_PORT_SCHEMA,
    ] {
        let path = agents_root()
            .join("contracts/port-contracts")
            .join(format!("{schema_id}.port-contract.json"));
        assert!(
            path.is_file(),
            "{schema_id} is admissible as a port binding but publishes no Port \
             Contract at {}",
            path.display()
        );
    }
}

/// Every vector file that publishes a document a carrier implements is listed
/// above. This is what stops a family from quietly losing its Rust reader while
/// keeping its schema reader.
#[test]
fn the_carrier_list_names_only_published_vector_files() {
    let root = agents_root().join("contracts/vectors");
    for file in FILES_WITH_A_CARRIER {
        assert!(
            root.join(file).is_file(),
            "{file} is claimed to have a carrier but publishes no vectors"
        );
    }
    let mut sorted = FILES_WITH_A_CARRIER.to_vec();
    sorted.sort_unstable();
    assert_eq!(
        sorted,
        FILES_WITH_A_CARRIER.to_vec(),
        "keep FILES_WITH_A_CARRIER sorted so additions are visible in review"
    );
}

/// `apxm.model-binding` publishes a `ResolvedModelBinding` whose field set the
/// Rust `ResolvedModelBinding` does not carry. The admission-side type that does
/// carry the joined proof is `AdmittedModelTarget`, and this pins how much of
/// the published document it actually accounts for.
#[test]
fn the_published_model_binding_is_accounted_for_by_the_admitted_model_target() {
    let published = load_schema("apxm.model-binding.json");
    let described: BTreeSet<String> = published["properties"]
        .as_object()
        .expect("published properties")
        .keys()
        .cloned()
        .collect();

    let admitted = serde_json::to_value(apxm_kernel::AdmittedModelTarget {
        model_target_ref: "model-target.1".into(),
        model_deployment_ref: "deployment.1".into(),
        exact_port_binding_digest: format!("sha256:{}", "1".repeat(64)),
    })
    .expect("encode");
    let carried: BTreeSet<String> = admitted
        .as_object()
        .expect("an object")
        .keys()
        .cloned()
        .collect();

    assert!(
        carried.is_subset(&described),
        "AdmittedModelTarget carries a field apxm.model-binding does not publish: \
         {:?}",
        carried.difference(&described).collect::<Vec<_>>()
    );
    let unaccounted: BTreeSet<&String> = described.difference(&carried).collect();
    assert_eq!(
        unaccounted
            .iter()
            .map(|field| field.as_str())
            .collect::<BTreeSet<_>>(),
        ["exact_port_binding_ref", "schema_version"]
            .into_iter()
            .collect::<BTreeSet<_>>(),
        "the published model binding and the admitted model target diverge by a \
         different set of fields than recorded"
    );
}
