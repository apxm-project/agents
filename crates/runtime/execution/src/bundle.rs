//! Immutable admission closure for the runtime kernel and driver-local ports.
//!
//! Durable event waiting and child Program composition are execution-driver
//! boundaries, so they cannot live in the lower kernel crate. This bundle
//! joins their exact bindings to the already-validated kernel bundle once at
//! construction and exposes no replacement or lookup operation afterward.

use std::sync::Arc;

use apxm_kernel::{BundleError, ExactPortBinding, PortBundle, PortSlot};
use apxm_program::artifact::SchemaDigestRef;

use crate::{CompositionPort, EventPort};

/// The complete immutable admitted port set required by the execution driver.
pub struct ExecutionPortBundle {
    kernel: Arc<PortBundle>,
    event_binding: ExactPortBinding,
    events: Arc<dyn EventPort>,
    composition_binding: ExactPortBinding,
    composition: Arc<dyn CompositionPort>,
}

impl std::fmt::Debug for ExecutionPortBundle {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ExecutionPortBundle")
            .field("kernel", &self.kernel)
            .field("event_binding", &self.event_binding)
            .field("composition_binding", &self.composition_binding)
            .finish_non_exhaustive()
    }
}

impl ExecutionPortBundle {
    /// Join exact driver bindings to one validated kernel bundle.
    ///
    /// # Errors
    ///
    /// Returns a closed [`BundleError`] when either driver binding names the
    /// wrong slot or differs from its expected exact Port Contract.
    pub fn construct(
        kernel: Arc<PortBundle>,
        expected_event_contract: SchemaDigestRef,
        event_binding: ExactPortBinding,
        events: Arc<dyn EventPort>,
        expected_composition_contract: SchemaDigestRef,
        composition_binding: ExactPortBinding,
        composition: Arc<dyn CompositionPort>,
    ) -> Result<Self, BundleError> {
        event_binding.validate_for(PortSlot::DurableEvent, &expected_event_contract)?;
        composition_binding
            .validate_for(PortSlot::ProgramComposition, &expected_composition_contract)?;
        Ok(Self {
            kernel,
            event_binding,
            events,
            composition_binding,
            composition,
        })
    }

    #[must_use]
    pub fn kernel(&self) -> &PortBundle {
        &self.kernel
    }

    #[must_use]
    pub fn events(&self) -> &Arc<dyn EventPort> {
        &self.events
    }

    #[must_use]
    pub fn composition(&self) -> &Arc<dyn CompositionPort> {
        &self.composition
    }

    #[must_use]
    pub fn event_binding(&self) -> &ExactPortBinding {
        &self.event_binding
    }

    #[must_use]
    pub fn composition_binding(&self) -> &ExactPortBinding {
        &self.composition_binding
    }
}

#[cfg(test)]
mod tests {
    use apxm_kernel::{
        ExecutionCommitPort, ExecutionCommitRequest, ExecutionCommitResult, PortBundleSpec,
        PortImplementation, ProgramInstanceRef,
    };
    use async_trait::async_trait;

    use super::*;
    use crate::{CompositionOutcome, CompositionRequest, EventAwait, EventOutcome, EventRef};

    fn digest(byte: u8) -> String {
        format!("sha256:{}", format!("{byte:02x}").repeat(32))
    }

    fn contract(schema_id: &str, byte: u8) -> SchemaDigestRef {
        SchemaDigestRef {
            schema_id: schema_id.into(),
            digest: digest(byte),
        }
    }

    fn binding(slot: PortSlot, contract: SchemaDigestRef, byte: u8) -> ExactPortBinding {
        ExactPortBinding {
            slot,
            port_contract: contract,
            binding_digest: digest(byte),
            proof_digest: digest(byte + 1),
        }
    }

    struct Commit;

    #[async_trait]
    impl ExecutionCommitPort for Commit {
        async fn commit(&self, _request: ExecutionCommitRequest) -> ExecutionCommitResult {
            ExecutionCommitResult::Committed {
                new_program_state_version: 1,
                evidence_position_ref: "evidence.1".into(),
            }
        }

        async fn current_version(&self, _program_instance_ref: &ProgramInstanceRef) -> u64 {
            0
        }

        /// The bundle fixture proves admission, not resumption.
        async fn load_continuation(
            &self,
            _program_instance_ref: &ProgramInstanceRef,
        ) -> Option<serde_json::Value> {
            None
        }
    }

    struct Events;

    #[async_trait]
    impl EventPort for Events {
        async fn await_event(&self, request: EventAwait) -> EventOutcome {
            EventOutcome::Fulfilled {
                event_ref: request.event_ref,
                payload: String::new(),
            }
        }
    }

    struct Composition;

    #[async_trait]
    impl CompositionPort for Composition {
        async fn program_new(&self, _request: CompositionRequest) -> CompositionOutcome {
            CompositionOutcome::Created {
                child_instance_ref: "child.1".into(),
            }
        }

        async fn program_invoke(&self, _request: CompositionRequest) -> CompositionOutcome {
            CompositionOutcome::Invoked {
                child_instance_ref: "child.1".into(),
            }
        }
    }

    fn kernel_bundle() -> Arc<PortBundle> {
        let execution_contract = contract("apxm.execution-commit.v1", 0x10);
        Arc::new(
            PortBundle::construct(
                &PortBundleSpec::new(vec![(
                    PortSlot::ExecutionCommit,
                    execution_contract.clone(),
                )]),
                vec![(
                    binding(PortSlot::ExecutionCommit, execution_contract, 0x11),
                    PortImplementation::ExecutionCommit(Arc::new(Commit)),
                )],
            )
            .expect("the focused kernel bundle is exact"),
        )
    }

    #[test]
    fn driver_ports_join_only_through_exact_bindings() {
        let event_contract = contract("apxm.durable-event.v1", 0x20);
        let composition_contract = contract("apxm.program-composition.v1", 0x30);
        let bundle = ExecutionPortBundle::construct(
            kernel_bundle(),
            event_contract.clone(),
            binding(PortSlot::DurableEvent, event_contract, 0x21),
            Arc::new(Events),
            composition_contract.clone(),
            binding(PortSlot::ProgramComposition, composition_contract, 0x31),
            Arc::new(Composition),
        )
        .expect("both driver bindings are exact");

        assert_eq!(bundle.event_binding().slot, PortSlot::DurableEvent);
        assert_eq!(
            bundle.composition_binding().slot,
            PortSlot::ProgramComposition
        );
        assert_eq!(
            EventRef::new("event.1").expect("event ref").as_str(),
            "event.1"
        );
    }

    #[test]
    fn driver_ports_reject_wrong_slots_and_contracts() {
        let event_contract = contract("apxm.durable-event.v1", 0x20);
        let composition_contract = contract("apxm.program-composition.v1", 0x30);
        let wrong_slot = ExecutionPortBundle::construct(
            kernel_bundle(),
            event_contract.clone(),
            binding(PortSlot::Capability, event_contract.clone(), 0x21),
            Arc::new(Events),
            composition_contract.clone(),
            binding(
                PortSlot::ProgramComposition,
                composition_contract.clone(),
                0x31,
            ),
            Arc::new(Composition),
        );
        assert!(matches!(
            wrong_slot,
            Err(BundleError::BindingSlotMismatch {
                expected: PortSlot::DurableEvent,
                actual: PortSlot::Capability,
            })
        ));

        let wrong_contract = ExecutionPortBundle::construct(
            kernel_bundle(),
            event_contract.clone(),
            binding(
                PortSlot::DurableEvent,
                contract("apxm.durable-event.v1", 0x22),
                0x21,
            ),
            Arc::new(Events),
            composition_contract.clone(),
            binding(PortSlot::ProgramComposition, composition_contract, 0x31),
            Arc::new(Composition),
        );
        assert!(matches!(
            wrong_contract,
            Err(BundleError::ContractMismatch {
                slot: PortSlot::DurableEvent,
                ..
            })
        ));
    }

    #[test]
    fn driver_requests_reject_unknown_wire_fields() {
        let event = serde_json::json!({
            "node_id": "node.await.1",
            "event_ref": "event.1",
            "provider": "first_available"
        });
        let composition = serde_json::json!({
            "node_id": "node.program.1",
            "receiver": {"program": {"program_ref": "program.1"}},
            "process": "spawn"
        });

        assert!(serde_json::from_value::<EventAwait>(event).is_err());
        assert!(serde_json::from_value::<CompositionRequest>(composition).is_err());
    }
}
