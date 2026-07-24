//! The typed port bundle, constructed from immutable Exact Port Bindings.
//!
//! The runtime validates proof, digest, slot-contract equality, and bundle
//! completeness at construction only. It does not admit, discover, resolve, or
//! rebind: a binding outside the declared spec is rejected, a missing slot is
//! rejected, and there is no method to search for or replace an implementation
//! after construction. Each first-party implementation enters through the same
//! exact binding — there is no generic registry or first-party bypass.

use std::collections::HashSet;
use std::sync::Arc;

use apxm_inference::ModelInferencePort;
use apxm_program::artifact::SchemaDigestRef;
use apxm_program::grammar::is_digest;

use crate::capability::CapabilityPort;
use crate::commit::ExecutionCommitPort;
use crate::confinement::ConfinementPort;
use crate::external_agent::ExternalAgentCapabilityPort;

/// The closed set of typed port slots a bundle can carry.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PortSlot {
    ExecutionCommit,
    Confinement,
    ModelInference,
    Capability,
    ExternalAgentCapability,
}

impl PortSlot {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ExecutionCommit => "execution_commit",
            Self::Confinement => "confinement",
            Self::ModelInference => "model_inference",
            Self::Capability => "capability",
            Self::ExternalAgentCapability => "external_agent_capability",
        }
    }
}

/// An immutable Exact Port Binding descriptor: the slot it fills, the exact port
/// contract it satisfies, its binding identity, and its admission proof.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExactPortBinding {
    pub slot: PortSlot,
    pub port_contract: SchemaDigestRef,
    pub binding_digest: String,
    pub proof_digest: String,
}

/// One provided implementation, tagged by the slot it fills.
pub enum PortImplementation {
    ExecutionCommit(Arc<dyn ExecutionCommitPort>),
    Confinement(Arc<dyn ConfinementPort>),
    // A model inference implementation admitted into a concurrent runtime bundle
    // must be thread-safe; the inference Port Contract itself is transport-neutral.
    ModelInference(Arc<dyn ModelInferencePort + Send + Sync>),
    Capability(Arc<dyn CapabilityPort>),
    ExternalAgentCapability(Arc<dyn ExternalAgentCapabilityPort>),
}

impl PortImplementation {
    #[must_use]
    pub fn slot(&self) -> PortSlot {
        match self {
            Self::ExecutionCommit(_) => PortSlot::ExecutionCommit,
            Self::Confinement(_) => PortSlot::Confinement,
            Self::ModelInference(_) => PortSlot::ModelInference,
            Self::Capability(_) => PortSlot::Capability,
            Self::ExternalAgentCapability(_) => PortSlot::ExternalAgentCapability,
        }
    }
}

/// The declared bundle requirement: each required slot and the exact port
/// contract it demands. `execution_commit` is always required.
#[derive(Clone, Debug)]
pub struct PortBundleSpec {
    required: Vec<(PortSlot, SchemaDigestRef)>,
}

impl PortBundleSpec {
    #[must_use]
    pub fn new(required: Vec<(PortSlot, SchemaDigestRef)>) -> Self {
        Self { required }
    }

    fn expected_contract(&self, slot: PortSlot) -> Option<&SchemaDigestRef> {
        self.required
            .iter()
            .find(|(s, _)| *s == slot)
            .map(|(_, contract)| contract)
    }
}

/// Why a bundle could not be constructed. Every variant fails closed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BundleError {
    /// A required slot had no provided binding.
    MissingSlot(PortSlot),
    /// A binding was provided for a slot the spec did not declare (no discovery).
    UnexpectedSlot(PortSlot),
    /// Two bindings filled the same slot.
    DuplicateSlot(PortSlot),
    /// The binding's port contract did not equal the slot's declared contract.
    ContractMismatch {
        slot: PortSlot,
        expected: SchemaDigestRef,
        actual: SchemaDigestRef,
    },
    /// The binding descriptor and the implementation named different slots.
    SlotImplementationMismatch(PortSlot),
    /// The binding digest was not a well-formed sha256 value.
    InvalidBindingDigest(PortSlot),
    /// The admission proof digest was not a well-formed sha256 value.
    InvalidProof(PortSlot),
    /// The spec did not require the mandatory execution-commit slot.
    ExecutionCommitNotRequired,
}

impl std::fmt::Display for BundleError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self:?}")
    }
}

impl std::error::Error for BundleError {}

/// A frozen, validated set of typed ports. There is no accessor that searches,
/// resolves, or rebinds; the ports are exactly those validated at construction.
pub struct PortBundle {
    execution_commit: Arc<dyn ExecutionCommitPort>,
    confinement: Option<Arc<dyn ConfinementPort>>,
    model_inference: Option<Arc<dyn ModelInferencePort + Send + Sync>>,
    capability: Option<Arc<dyn CapabilityPort>>,
    external_agent_capability: Option<Arc<dyn ExternalAgentCapabilityPort>>,
}

impl std::fmt::Debug for PortBundle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PortBundle")
            .field("execution_commit", &true)
            .field("confinement", &self.confinement.is_some())
            .field("model_inference", &self.model_inference.is_some())
            .field("capability", &self.capability.is_some())
            .field(
                "external_agent_capability",
                &self.external_agent_capability.is_some(),
            )
            .finish()
    }
}

impl PortBundle {
    /// Construct a bundle by validating each provided binding against the spec.
    /// Validation happens only here; the result is immutable.
    ///
    /// # Errors
    ///
    /// Returns a [`BundleError`] on any missing/unexpected/duplicate slot,
    /// contract mismatch, slot/implementation mismatch, or malformed digest.
    pub fn construct(
        spec: &PortBundleSpec,
        entries: Vec<(ExactPortBinding, PortImplementation)>,
    ) -> Result<Self, BundleError> {
        if spec.expected_contract(PortSlot::ExecutionCommit).is_none() {
            return Err(BundleError::ExecutionCommitNotRequired);
        }

        let mut execution_commit: Option<Arc<dyn ExecutionCommitPort>> = None;
        let mut confinement: Option<Arc<dyn ConfinementPort>> = None;
        let mut model_inference: Option<Arc<dyn ModelInferencePort + Send + Sync>> = None;
        let mut capability: Option<Arc<dyn CapabilityPort>> = None;
        let mut external_agent_capability: Option<Arc<dyn ExternalAgentCapabilityPort>> = None;
        let mut seen: HashSet<PortSlot> = HashSet::new();

        for (binding, implementation) in entries {
            if binding.slot != implementation.slot() {
                return Err(BundleError::SlotImplementationMismatch(binding.slot));
            }
            let Some(expected) = spec.expected_contract(binding.slot) else {
                return Err(BundleError::UnexpectedSlot(binding.slot));
            };
            if !seen.insert(binding.slot) {
                return Err(BundleError::DuplicateSlot(binding.slot));
            }
            if binding.port_contract != *expected {
                return Err(BundleError::ContractMismatch {
                    slot: binding.slot,
                    expected: expected.clone(),
                    actual: binding.port_contract,
                });
            }
            if !is_digest(&binding.binding_digest) {
                return Err(BundleError::InvalidBindingDigest(binding.slot));
            }
            if !is_digest(&binding.proof_digest) {
                return Err(BundleError::InvalidProof(binding.slot));
            }

            match implementation {
                PortImplementation::ExecutionCommit(port) => execution_commit = Some(port),
                PortImplementation::Confinement(port) => confinement = Some(port),
                PortImplementation::ModelInference(port) => model_inference = Some(port),
                PortImplementation::Capability(port) => capability = Some(port),
                PortImplementation::ExternalAgentCapability(port) => {
                    external_agent_capability = Some(port);
                }
            }
        }

        for (slot, _) in &spec.required {
            if !seen.contains(slot) {
                return Err(BundleError::MissingSlot(*slot));
            }
        }

        let execution_commit =
            execution_commit.ok_or(BundleError::MissingSlot(PortSlot::ExecutionCommit))?;

        Ok(Self {
            execution_commit,
            confinement,
            model_inference,
            capability,
            external_agent_capability,
        })
    }

    #[must_use]
    pub fn execution_commit(&self) -> &Arc<dyn ExecutionCommitPort> {
        &self.execution_commit
    }

    #[must_use]
    pub fn confinement(&self) -> Option<&Arc<dyn ConfinementPort>> {
        self.confinement.as_ref()
    }

    #[must_use]
    pub fn model_inference(&self) -> Option<&Arc<dyn ModelInferencePort + Send + Sync>> {
        self.model_inference.as_ref()
    }

    #[must_use]
    pub fn capability(&self) -> Option<&Arc<dyn CapabilityPort>> {
        self.capability.as_ref()
    }

    #[must_use]
    pub fn external_agent_capability(&self) -> Option<&Arc<dyn ExternalAgentCapabilityPort>> {
        self.external_agent_capability.as_ref()
    }
}
