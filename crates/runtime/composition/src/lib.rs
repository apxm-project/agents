//! The agents runtime Composition Root.
//!
//! It assembles the canonical runtime from admitted Exact Port Bindings and the
//! first-party adapter implementations: the atomic Execution Commit, Confinement,
//! vLLM model inference, and External Agent (ACP) ports. It validates the bundle
//! at construction — proof, digest, slot-contract equality, completeness — and
//! never discovers, ranks, or rebinds. The assembled ports drive the canonical
//! `apxm-execution` driver.
//!
//! The durable Execution Commit backend is Server-owned; the in-memory
//! implementation here is the deterministic standalone/test variant. Live vLLM
//! and ACP transports are injected, so this root wires the exact production
//! topology while remaining testable against deterministic fake transports.

use std::sync::Arc;

use apxm_execution::{
    CapabilityPort, CompositionPort, EventPort, ExecutionPorts, ExecutionPortsError,
    StaticHookHandlerPort,
};
use apxm_inference::ModelInferencePort;
use apxm_kernel::{
    BundleError, ConfinementPort, ExactPortBinding, ExecutionCommitPort,
    ExternalAgentCapabilityPort, PortBundle, PortBundleSpec, PortImplementation, PortSlot,
};

use apxm_program::artifact::SchemaDigestRef;

/// The exact admitted implementations the Composition Root wires. Each is a
/// single injected implementation entering through its own Exact Port Binding.
pub struct AdmittedPorts {
    pub execution_commit: Arc<dyn ExecutionCommitPort>,
    pub confinement: Arc<dyn ConfinementPort>,
    pub model_inference: Arc<dyn ModelInferencePort + Send + Sync>,
    pub capability: Arc<dyn CapabilityPort>,
    pub external_agent: Arc<dyn ExternalAgentCapabilityPort>,
    pub hook_handlers: Arc<dyn StaticHookHandlerPort>,
}

/// One admitted binding descriptor paired with the port contract it satisfies.
pub struct AdmittedBinding {
    pub slot: PortSlot,
    pub port_contract: SchemaDigestRef,
    pub binding: ExactPortBinding,
}

/// Assemble a validated kernel [`PortBundle`] from the admitted bindings and the
/// first-party adapter implementations.
///
/// # Errors
///
/// Returns a [`BundleError`] if the bindings are incomplete, mismatched, or carry
/// a malformed digest — construction fails closed.
pub fn assemble_bundle(
    bindings: [AdmittedBinding; 5],
    ports: &AdmittedPorts,
) -> Result<PortBundle, BundleError> {
    let spec = PortBundleSpec::new(
        bindings
            .iter()
            .map(|b| (b.slot, b.port_contract.clone()))
            .collect(),
    );
    let [commit_b, confine_b, model_b, capability_b, acp_b] = bindings;
    PortBundle::construct(
        &spec,
        vec![
            (
                commit_b.binding,
                PortImplementation::ExecutionCommit(ports.execution_commit.clone()),
            ),
            (
                confine_b.binding,
                PortImplementation::Confinement(ports.confinement.clone()),
            ),
            (
                model_b.binding,
                PortImplementation::ModelInference(ports.model_inference.clone()),
            ),
            (
                capability_b.binding,
                PortImplementation::Capability(ports.capability.clone()),
            ),
            (
                acp_b.binding,
                PortImplementation::ExternalAgentCapability(ports.external_agent.clone()),
            ),
        ],
    )
}

/// Assemble the canonical driver's [`ExecutionPorts`] from one validated bundle
/// plus the non-admitting driver-local ports.
///
/// # Errors
///
/// Returns [`ExecutionPortsError`] when the bundle omitted an effect port the
/// driver must dispatch through an exact admitted binding.
pub fn execution_ports(
    bundle: &PortBundle,
    hook_handlers: Arc<dyn StaticHookHandlerPort>,
    events: Arc<dyn EventPort>,
    composition: Arc<dyn CompositionPort>,
) -> Result<ExecutionPorts, ExecutionPortsError> {
    ExecutionPorts::from_admitted_bundle(bundle, events, composition, hook_handlers)
}
