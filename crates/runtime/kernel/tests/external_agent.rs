//! External Agent (ACP) capability vectors: an admitted peer profile prompt is
//! one outer capability.invoke NodeExecution with the peer loop as nested
//! attributed evidence, and peer usage never enters native model accounting.
//! Driven by a deterministic scripted peer and commit fixture (test doubles).

use std::sync::{Arc, Mutex};

use async_trait::async_trait;

use apxm_program::artifact::SchemaDigestRef;
use apxm_program::external_agent::{AttributedEvent, AttributedEventKind, PeerUsage};
use apxm_program::runtime_evidence::{Fact, FactKind, ProgramIdentity};

use apxm_kernel::{
    AcpPromptOutcome, AcpPromptRequest, AtomicWriteSet, CapabilityInvocation, ExactPortBinding,
    ExecutionCommitPort, ExecutionCommitRequest, ExecutionCommitResult,
    ExternalAgentCapabilityPort, InstanceError, InvocationReport, PortBundle, PortBundleSpec,
    PortImplementation, PortSlot, ProgramInstance, ProgramInstanceRef, ProgramInvocationRef,
    PromptEffectState, reconstruct,
};

fn digest(c: char) -> String {
    format!("sha256:{}", c.to_string().repeat(64))
}

fn write_set() -> AtomicWriteSet {
    AtomicWriteSet {
        next_program_state_digest: digest('1'),
        continuation_digest: digest('2'),
        checkpoint_effect_outcomes_digest: digest('3'),
        runtime_evidence_batch_digest: apxm_kernel::runtime_evidence_and_observation_digest(
            &[],
            &[],
        ),
        usage_facts_digest: digest('5'),
        session_output_refs_digest: apxm_kernel::session_output_refs_digest(&[]),
    }
}

fn identity() -> ProgramIdentity {
    ProgramIdentity {
        artifact_digest: digest('a'),
        entrypoint: "run".into(),
        agent_identity_binding: "agent.1".into(),
        program_instance_id: Some("instance.1".into()),
    }
}

fn contract() -> SchemaDigestRef {
    SchemaDigestRef {
        schema_id: "apxm.execution-commit".into(),
        digest: digest('e'),
    }
}

fn acp_contract() -> SchemaDigestRef {
    SchemaDigestRef {
        schema_id: "apxm.external-agent-session".into(),
        digest: digest('f'),
    }
}

struct FixtureCommit {
    state: Mutex<(u64, Vec<Fact>)>,
}

impl FixtureCommit {
    fn new() -> Self {
        Self {
            state: Mutex::new((0, Vec::new())),
        }
    }
    fn evidence(&self) -> Vec<Fact> {
        self.state.lock().unwrap().1.clone()
    }
}

#[async_trait]
impl ExecutionCommitPort for FixtureCommit {
    async fn commit(&self, request: ExecutionCommitRequest) -> ExecutionCommitResult {
        let mut state = self.state.lock().unwrap();
        state.0 = request.expected_program_state_version + 1;
        state.1.extend(request.evidence_batch);
        ExecutionCommitResult::Committed {
            new_program_state_version: state.0,
            evidence_position_ref: "evidence:1".into(),
        }
    }
    async fn current_version(&self, _program_instance_ref: &ProgramInstanceRef) -> u64 {
        self.state.lock().unwrap().0
    }

    /// This fixture never parks, so it holds no committed continuation.
    async fn load_continuation(
        &self,
        _program_instance_ref: &ProgramInstanceRef,
    ) -> Option<serde_json::Value> {
        None
    }
}

struct ScriptedAcpPeer {
    outcome: AcpPromptOutcome,
}

#[async_trait]
impl ExternalAgentCapabilityPort for ScriptedAcpPeer {
    async fn prompt(&self, request: AcpPromptRequest) -> AcpPromptOutcome {
        let mut outcome = self.outcome.clone();
        outcome.session_ref = request.session_ref;
        outcome
    }
}

fn event(seq: u64, kind: AttributedEventKind) -> AttributedEvent {
    AttributedEvent {
        event_sequence: seq,
        kind,
        detail: None,
        reverse_operation: None,
        reverse_target: None,
        reverse_decision: None,
    }
}

fn instance(commit: Arc<FixtureCommit>, peer: Arc<ScriptedAcpPeer>) -> ProgramInstance {
    instance_with_peer(commit, peer)
}

fn instance_with_peer(
    commit: Arc<FixtureCommit>,
    peer: Arc<dyn ExternalAgentCapabilityPort>,
) -> ProgramInstance {
    let spec = PortBundleSpec::new(vec![
        (PortSlot::ExecutionCommit, contract()),
        (PortSlot::ExternalAgentCapability, acp_contract()),
    ]);
    let bundle = PortBundle::construct(
        &spec,
        vec![
            (
                ExactPortBinding {
                    slot: PortSlot::ExecutionCommit,
                    port_contract: contract(),
                    binding_digest: digest('b'),
                    proof_digest: digest('c'),
                },
                PortImplementation::ExecutionCommit(commit),
            ),
            (
                ExactPortBinding {
                    slot: PortSlot::ExternalAgentCapability,
                    port_contract: acp_contract(),
                    binding_digest: digest('d'),
                    proof_digest: digest('9'),
                },
                PortImplementation::ExternalAgentCapability(peer),
            ),
        ],
    )
    .expect("valid bundle");
    ProgramInstance::new(identity(), ProgramInstanceRef::new("instance.1"), bundle)
        .expect("Program Instance identity matches its commit key")
}

struct NeverCalledPeer;

#[async_trait]
impl ExternalAgentCapabilityPort for NeverCalledPeer {
    async fn prompt(&self, _request: AcpPromptRequest) -> AcpPromptOutcome {
        panic!("invalid atomic commit input must not dispatch an external effect")
    }
}

fn invocation(profile: &str) -> CapabilityInvocation {
    CapabilityInvocation {
        commit_id: "c1".into(),
        program_invocation_ref: ProgramInvocationRef::new("invocation.acp.1"),
        capability_node_execution_id: "nodeexec.cap.1".into(),
        request: AcpPromptRequest {
            effect_ref: "effect.1".into(),
            session_ref: "session.acp.1".into(),
            profile_ref: profile.into(),
            prompt: "delegate".into(),
        },
        write_set: write_set(),
    }
}

#[test]
fn external_agent_request_rejects_unknown_wire_fields() {
    let mut value =
        serde_json::to_value(invocation("acp:codex").request).expect("serialize exact request");
    value
        .as_object_mut()
        .expect("request is an object")
        .insert("executable".into(), serde_json::json!("codex"));

    assert!(serde_json::from_value::<AcpPromptRequest>(value).is_err());
}

async fn run_profile(profile: &str, reported_value: &str) -> apxm_kernel::CapabilityReport {
    let commit = Arc::new(FixtureCommit::new());
    let peer = Arc::new(ScriptedAcpPeer {
        outcome: AcpPromptOutcome {
            session_ref: String::new(),
            state: PromptEffectState::Completed {
                stop_reason: Some("end_turn".into()),
            },
            nested_events: vec![
                event(0, AttributedEventKind::Message),
                event(1, AttributedEventKind::ToolCall),
                event(2, AttributedEventKind::ToolCall),
                event(3, AttributedEventKind::Message),
            ],
            peer_usage: vec![PeerUsage {
                reported_by: profile.to_string(),
                metric_scope: "peer.tokens.total".into(),
                reported_value: reported_value.into(),
                availability_partial: None,
            }],
        },
    });
    let inst = instance(commit.clone(), peer);
    let report = inst
        .invoke_capability(invocation(profile))
        .await
        .expect("capability runs");
    // The durable runtime evidence carries the one Capability NodeExecution and
    // no native model outcome or native usage.
    let facts = commit.evidence();
    assert!(
        facts.iter().all(|fact| fact
            .runtime()
            .is_none_or(|runtime| runtime.model_outcome.is_none())),
        "no native model outcome is fabricated for an ACP peer loop",
    );
    assert!(
        facts.iter().any(|fact| fact
            .runtime()
            .and_then(|runtime| runtime.node_execution_id.as_deref())
            == Some("nodeexec.cap.1")),
        "the capability NodeExecution is recorded",
    );
    report
}

#[tokio::test]
async fn claude_code_prompt_is_one_capability_with_nested_peer_loop() {
    let report = run_profile("acp:claude-code", "12345").await;
    assert_eq!(
        report.commit,
        InvocationReport::Committed {
            new_program_state_version: 1
        }
    );
    // One outer capability, the peer loop nested under it.
    assert_eq!(
        report.evidence.capability_node_execution_id,
        "nodeexec.cap.1"
    );
    assert_eq!(report.evidence.attributed_events.len(), 4);
    assert!(report.evidence.verify().is_accepted());
    // Peer usage is provenance, preserved verbatim as an opaque string.
    assert_eq!(report.evidence.peer_usage.len(), 1);
    assert_eq!(report.evidence.peer_usage[0].reported_value, "12345");
    assert_eq!(
        report.evidence.peer_usage[0].metric_scope,
        "peer.tokens.total"
    );
}

#[tokio::test]
async fn codex_prompt_is_one_capability_with_nested_peer_loop() {
    let report = run_profile("acp:codex", "67890").await;
    assert_eq!(
        report.commit,
        InvocationReport::Committed {
            new_program_state_version: 1
        }
    );
    assert_eq!(report.evidence.attributed_events.len(), 4);
    assert_eq!(report.evidence.peer_usage[0].reported_by, "acp:codex");
    assert_eq!(report.evidence.peer_usage[0].reported_value, "67890");
}

#[tokio::test]
async fn peer_usage_never_becomes_native_tokens() {
    let report = run_profile("acp:claude-code", "999999").await;
    // The measurement stays an opaque string in third-party provenance; there is
    // no native token field on peer usage and no native usage fact derived here.
    assert_eq!(report.evidence.peer_usage[0].reported_value, "999999");
    assert!(report.evidence.verify().is_accepted());
}

#[tokio::test]
async fn outcome_unknown_transport_records_uncertain_not_success() {
    let commit = Arc::new(FixtureCommit::new());
    let peer = Arc::new(ScriptedAcpPeer {
        outcome: AcpPromptOutcome {
            session_ref: String::new(),
            state: PromptEffectState::OutcomeUnknown {
                message: "reconcile:acp.1".into(),
            },
            nested_events: vec![event(0, AttributedEventKind::Message)],
            peer_usage: vec![],
        },
    });
    let inst = instance(commit.clone(), peer);
    let report = inst
        .invoke_capability(invocation("acp:codex"))
        .await
        .expect("runs");
    assert!(matches!(
        report.commit,
        InvocationReport::OutcomeUnknown { .. }
    ));

    let evidence = apxm_program::runtime_evidence::RuntimeEvidence {
        schema_version: apxm_program::runtime_evidence::RuntimeEvidenceVersion::V1,
        program_identity: identity(),
        facts: commit.evidence(),
    };
    let view = reconstruct(&evidence);
    assert!(
        !view.committed,
        "an uncertain transport never becomes a committed success"
    );
    assert!(view.outcome_unknown);
    assert!(
        commit
            .evidence()
            .iter()
            .any(|fact| fact.is_kind(FactKind::EffectOutcomeUnknown)),
        "the uncertain effect is recorded honestly",
    );
}

#[tokio::test]
async fn missing_external_agent_port_fails_closed() {
    // A bundle without the external-agent slot cannot run an ACP capability.
    let commit = Arc::new(FixtureCommit::new());
    let spec = PortBundleSpec::new(vec![(PortSlot::ExecutionCommit, contract())]);
    let bundle = PortBundle::construct(
        &spec,
        vec![(
            ExactPortBinding {
                slot: PortSlot::ExecutionCommit,
                port_contract: contract(),
                binding_digest: digest('b'),
                proof_digest: digest('c'),
            },
            PortImplementation::ExecutionCommit(commit),
        )],
    )
    .expect("valid bundle");
    let inst = ProgramInstance::new(identity(), ProgramInstanceRef::new("instance.1"), bundle)
        .expect("Program Instance identity matches its commit key");
    let err = inst
        .invoke_capability(invocation("acp:claude-code"))
        .await
        .expect_err("no acp port");
    assert_eq!(
        err,
        InstanceError::MissingPort(PortSlot::ExternalAgentCapability)
    );
}

#[tokio::test]
async fn invalid_atomic_commit_input_is_rejected_before_external_effect() {
    let commit = Arc::new(FixtureCommit::new());
    let inst = instance_with_peer(commit.clone(), Arc::new(NeverCalledPeer));
    let mut request = invocation("acp:codex");
    request.write_set.session_output_refs_digest = "not-a-digest".into();

    let error = inst
        .invoke_capability(request)
        .await
        .expect_err("malformed commit input");
    assert!(matches!(error, InstanceError::InvalidCommitRequest { .. }));
    assert!(commit.evidence().is_empty());
}
