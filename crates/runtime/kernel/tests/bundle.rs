//! Port bundle construction vectors: the runtime validates proof, digest,
//! slot-contract equality, and completeness at construction only, and fails
//! closed on any mismatch. It never discovers, resolves, or rebinds.

use std::sync::Arc;

use async_trait::async_trait;

use apxm_inference::{AttemptDisposition, ModelCallRequest, ModelInferencePort};
use apxm_program::artifact::SchemaDigestRef;

use apxm_kernel::{
    BundleError, CapabilityOutcome, CapabilityPort, CapabilityRequest, ConfinementAttestation,
    ConfinementError, ConfinementPort, ConfinementRequest, ExactPortBinding, ExecutionCommitPort,
    ExecutionCommitRequest, ExecutionCommitResult, PortBundle, PortBundleSpec, PortImplementation,
    PortSlot, ProgramInstanceRef,
};

fn digest(c: char) -> String {
    format!("sha256:{}", c.to_string().repeat(64))
}

fn commit_contract() -> SchemaDigestRef {
    SchemaDigestRef {
        schema_id: "apxm.execution-commit.v1".into(),
        digest: digest('e'),
    }
}

struct NoopCommit;

#[async_trait]
impl ExecutionCommitPort for NoopCommit {
    async fn commit(&self, _request: ExecutionCommitRequest) -> ExecutionCommitResult {
        ExecutionCommitResult::CompareConflict {
            current_program_state_version: 0,
        }
    }
    async fn current_version(&self, _program_instance_ref: &ProgramInstanceRef) -> u64 {
        0
    }

    /// This fixture never parks, so it holds no committed continuation.
    async fn load_continuation(
        &self,
        _program_instance_ref: &ProgramInstanceRef,
    ) -> Option<serde_json::Value> {
        None
    }
}

struct NoopConfinement;

#[async_trait]
impl ConfinementPort for NoopConfinement {
    async fn attest(
        &self,
        _request: ConfinementRequest,
    ) -> Result<ConfinementAttestation, ConfinementError> {
        Err(ConfinementError::MalformedSandboxDigest)
    }
}

struct NoopInference;

impl ModelInferencePort for NoopInference {
    fn attempt(&self, _request: &ModelCallRequest, _attempt: u32) -> AttemptDisposition {
        AttemptDisposition::Cancelled
    }
}

struct NoopCapability;

#[async_trait]
impl CapabilityPort for NoopCapability {
    async fn invoke(&self, _request: CapabilityRequest) -> CapabilityOutcome {
        CapabilityOutcome::OutcomeUnknown {
            message: "test capability is not invoked".into(),
        }
    }
}

fn commit_binding(contract: SchemaDigestRef, binding: char, proof: char) -> ExactPortBinding {
    ExactPortBinding {
        slot: PortSlot::ExecutionCommit,
        port_contract: contract,
        binding_digest: digest(binding),
        proof_digest: digest(proof),
    }
}

fn commit_spec() -> PortBundleSpec {
    PortBundleSpec::new(vec![(PortSlot::ExecutionCommit, commit_contract())])
}

fn commit_impl() -> PortImplementation {
    PortImplementation::ExecutionCommit(Arc::new(NoopCommit))
}

#[test]
fn valid_bundle_constructs() {
    let bundle = PortBundle::construct(
        &commit_spec(),
        vec![(commit_binding(commit_contract(), 'b', 'c'), commit_impl())],
    )
    .expect("valid bundle");
    assert!(bundle.confinement().is_none());
    assert!(bundle.model_inference().is_none());
}

#[test]
fn missing_required_slot_fails_closed() {
    let spec = PortBundleSpec::new(vec![
        (PortSlot::ExecutionCommit, commit_contract()),
        (PortSlot::Confinement, commit_contract()),
    ]);
    let err = PortBundle::construct(
        &spec,
        vec![(commit_binding(commit_contract(), 'b', 'c'), commit_impl())],
    )
    .expect_err("missing confinement slot");
    assert_eq!(err, BundleError::MissingSlot(PortSlot::Confinement));
}

#[test]
fn unexpected_slot_is_rejected_not_discovered() {
    let confinement_binding = ExactPortBinding {
        slot: PortSlot::Confinement,
        port_contract: commit_contract(),
        binding_digest: digest('b'),
        proof_digest: digest('c'),
    };
    let err = PortBundle::construct(
        &commit_spec(),
        vec![
            (commit_binding(commit_contract(), 'b', 'c'), commit_impl()),
            (
                confinement_binding,
                PortImplementation::Confinement(Arc::new(NoopConfinement)),
            ),
        ],
    )
    .expect_err("undeclared slot");
    assert_eq!(err, BundleError::UnexpectedSlot(PortSlot::Confinement));
}

#[test]
fn contract_digest_mismatch_fails_closed() {
    let wrong = SchemaDigestRef {
        schema_id: "apxm.execution-commit.v1".into(),
        digest: digest('f'),
    };
    let err = PortBundle::construct(
        &commit_spec(),
        vec![(commit_binding(wrong, 'b', 'c'), commit_impl())],
    )
    .expect_err("contract mismatch");
    assert!(matches!(err, BundleError::ContractMismatch { .. }));
}

#[test]
fn slot_implementation_mismatch_fails_closed() {
    // Binding names ExecutionCommit but the implementation is Confinement.
    let err = PortBundle::construct(
        &commit_spec(),
        vec![(
            commit_binding(commit_contract(), 'b', 'c'),
            PortImplementation::Confinement(Arc::new(NoopConfinement)),
        )],
    )
    .expect_err("slot/impl mismatch");
    assert_eq!(
        err,
        BundleError::SlotImplementationMismatch(PortSlot::ExecutionCommit)
    );
}

#[test]
fn malformed_binding_or_proof_digest_fails_closed() {
    let bad_binding = ExactPortBinding {
        slot: PortSlot::ExecutionCommit,
        port_contract: commit_contract(),
        binding_digest: "not-a-digest".into(),
        proof_digest: digest('c'),
    };
    let err = PortBundle::construct(&commit_spec(), vec![(bad_binding, commit_impl())])
        .expect_err("bad binding digest");
    assert_eq!(
        err,
        BundleError::InvalidBindingDigest(PortSlot::ExecutionCommit)
    );

    let bad_proof = ExactPortBinding {
        slot: PortSlot::ExecutionCommit,
        port_contract: commit_contract(),
        binding_digest: digest('b'),
        proof_digest: "no-proof".into(),
    };
    let err = PortBundle::construct(&commit_spec(), vec![(bad_proof, commit_impl())])
        .expect_err("bad proof digest");
    assert_eq!(err, BundleError::InvalidProof(PortSlot::ExecutionCommit));
}

#[test]
fn duplicate_slot_fails_closed() {
    let err = PortBundle::construct(
        &commit_spec(),
        vec![
            (commit_binding(commit_contract(), 'b', 'c'), commit_impl()),
            (commit_binding(commit_contract(), 'd', 'e'), commit_impl()),
        ],
    )
    .expect_err("duplicate slot");
    assert_eq!(err, BundleError::DuplicateSlot(PortSlot::ExecutionCommit));
}

#[test]
fn model_inference_slot_binds_when_declared() {
    let spec = PortBundleSpec::new(vec![
        (PortSlot::ExecutionCommit, commit_contract()),
        (PortSlot::ModelInference, commit_contract()),
    ]);
    let inference_binding = ExactPortBinding {
        slot: PortSlot::ModelInference,
        port_contract: commit_contract(),
        binding_digest: digest('d'),
        proof_digest: digest('f'),
    };
    let bundle = PortBundle::construct(
        &spec,
        vec![
            (commit_binding(commit_contract(), 'b', 'c'), commit_impl()),
            (
                inference_binding,
                PortImplementation::ModelInference(Arc::new(NoopInference)),
            ),
        ],
    )
    .expect("valid bundle with inference");
    assert!(bundle.model_inference().is_some());
}

#[test]
fn spec_without_execution_commit_fails_closed() {
    let spec = PortBundleSpec::new(vec![(PortSlot::Confinement, commit_contract())]);
    let err = PortBundle::construct(&spec, vec![]).expect_err("execution commit required");
    assert_eq!(err, BundleError::ExecutionCommitNotRequired);
}

#[test]
fn capability_slot_binds_only_when_exactly_declared() {
    let spec = PortBundleSpec::new(vec![
        (PortSlot::ExecutionCommit, commit_contract()),
        (PortSlot::Capability, commit_contract()),
    ]);
    let capability_binding = ExactPortBinding {
        slot: PortSlot::Capability,
        port_contract: commit_contract(),
        binding_digest: digest('d'),
        proof_digest: digest('f'),
    };
    let bundle = PortBundle::construct(
        &spec,
        vec![
            (commit_binding(commit_contract(), 'b', 'c'), commit_impl()),
            (
                capability_binding,
                PortImplementation::Capability(Arc::new(NoopCapability)),
            ),
        ],
    )
    .expect("valid bundle with a capability");
    assert!(bundle.capability().is_some());
}

#[test]
fn capability_slot_rejects_missing_unexpected_mismatched_and_wrong_contract_bindings() {
    let spec = PortBundleSpec::new(vec![
        (PortSlot::ExecutionCommit, commit_contract()),
        (PortSlot::Capability, commit_contract()),
    ]);
    let missing = PortBundle::construct(
        &spec,
        vec![(commit_binding(commit_contract(), 'b', 'c'), commit_impl())],
    )
    .expect_err("capability binding is required");
    assert_eq!(missing, BundleError::MissingSlot(PortSlot::Capability));

    let unexpected = PortBundle::construct(
        &commit_spec(),
        vec![
            (commit_binding(commit_contract(), 'b', 'c'), commit_impl()),
            (
                ExactPortBinding {
                    slot: PortSlot::Capability,
                    port_contract: commit_contract(),
                    binding_digest: digest('d'),
                    proof_digest: digest('f'),
                },
                PortImplementation::Capability(Arc::new(NoopCapability)),
            ),
        ],
    )
    .expect_err("capability binding cannot be supplied outside the spec");
    assert_eq!(
        unexpected,
        BundleError::UnexpectedSlot(PortSlot::Capability)
    );

    let mismatched = PortBundle::construct(
        &spec,
        vec![
            (commit_binding(commit_contract(), 'b', 'c'), commit_impl()),
            (
                ExactPortBinding {
                    slot: PortSlot::Capability,
                    port_contract: commit_contract(),
                    binding_digest: digest('d'),
                    proof_digest: digest('f'),
                },
                PortImplementation::Confinement(Arc::new(NoopConfinement)),
            ),
        ],
    )
    .expect_err("capability binding must carry a capability implementation");
    assert_eq!(
        mismatched,
        BundleError::SlotImplementationMismatch(PortSlot::Capability)
    );

    let wrong_contract = SchemaDigestRef {
        schema_id: "apxm.capability-invocation.v1".into(),
        digest: digest('f'),
    };
    let mismatch = PortBundle::construct(
        &spec,
        vec![
            (commit_binding(commit_contract(), 'b', 'c'), commit_impl()),
            (
                ExactPortBinding {
                    slot: PortSlot::Capability,
                    port_contract: wrong_contract,
                    binding_digest: digest('d'),
                    proof_digest: digest('f'),
                },
                PortImplementation::Capability(Arc::new(NoopCapability)),
            ),
        ],
    )
    .expect_err("capability contract must match the declared contract");
    assert!(matches!(
        mismatch,
        BundleError::ContractMismatch {
            slot: PortSlot::Capability,
            ..
        }
    ));
}
