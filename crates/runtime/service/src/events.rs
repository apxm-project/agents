//! Reservation authority and the Event side of the shared continuation worker.
//! The index is a read projection of committed metadata, never a second writer.

use super::*;
use apxm_kernel::event_api::EventWaitBinding;
use apxm_kernel::{
    EventAwait, EventDelivery, EventOutcome, EventPort, EventRef, EventWaitContract,
    EventWaitRejection,
};
use std::sync::RwLock;

#[derive(Clone, Default)]
pub(super) struct EventIndex(Arc<RwLock<BTreeMap<(String, u64), IndexedReservation>>>);

#[derive(Clone)]
struct IndexedReservation {
    program_instance_id: String,
    type_id: String,
    schema_digest: String,
    binding: Option<EventWaitBinding>,
}

struct ReservedEvents {
    program_instance_id: String,
    index: EventIndex,
}

#[async_trait]
impl EventPort for ReservedEvents {
    async fn await_event(&self, request: EventAwait) -> EventOutcome {
        let reject = |reason| EventOutcome::Rejected { reason };
        let (reference, type_id, schema_digest) = match (&request.event_ref, &request.contract) {
            (EventRef::HostCapability { request_id }, EventWaitContract::HostCapability)
                if is_host_capability_request_id(request_id) =>
            {
                return EventOutcome::Parked;
            }
            (
                EventRef::Reserved { reference },
                EventWaitContract::Declared {
                    type_id,
                    schema_digest,
                },
            ) => (reference, type_id, schema_digest),
            _ => return reject(EventWaitRejection::ContractMismatch),
        };
        if reference.validate().is_err() {
            return reject(EventWaitRejection::UnknownReference);
        }
        let Ok(index) = self.index.0.read() else {
            return reject(EventWaitRejection::UnknownReference);
        };
        let Some(reservation) = index.get(&(reference.event_id.clone(), reference.generation))
        else {
            return reject(EventWaitRejection::UnknownReference);
        };
        if reservation.program_instance_id != self.program_instance_id {
            return reject(EventWaitRejection::OwnerMismatch);
        }
        if &reservation.type_id != type_id || &reservation.schema_digest != schema_digest {
            return reject(EventWaitRejection::ContractMismatch);
        }
        let binding = EventWaitBinding {
            program_instance_id: self.program_instance_id.clone(),
            program_invocation_id: request.program_invocation_id,
            node_id: request.node_id,
            node_execution_id: request.node_execution_id,
            event_ref: reference.clone(),
        };
        if reservation
            .binding
            .as_ref()
            .is_some_and(|prior| prior != &binding)
        {
            return reject(EventWaitRejection::AlreadyBound);
        }
        // Even early delivery first commits the exact wait. The dispatcher
        // then observes the durable terminal and runs its ordinary resume lease.
        EventOutcome::Parked
    }
}

impl RuntimeService {
    pub(super) fn expire_pending_events(&mut self) -> Result<(), String> {
        let now = Instant::now();
        let expired = self
            .reservations
            .iter()
            .filter(|(_, state)| {
                state.status == EventStatus::Pending && state.state_entry.expired(now)
            })
            .map(|(key, _)| key.clone())
            .collect::<Vec<_>>();
        if expired.is_empty() {
            return Ok(());
        }
        for key in &expired {
            self.reservations.get_mut(key).unwrap().status = EventStatus::Expired;
        }
        if let Err(error) = self.persist_runtime_state() {
            for key in expired {
                self.reservations.get_mut(&key).unwrap().status = EventStatus::Pending;
            }
            return Err(error);
        }
        Ok(())
    }

    pub(super) fn refresh_event_index(&self) -> Result<(), String> {
        let projection = self
            .reservations
            .iter()
            .map(|(key, reservation)| {
                (
                    key.clone(),
                    IndexedReservation {
                        program_instance_id: reservation.program_instance_id.clone(),
                        type_id: reservation.type_id.clone(),
                        schema_digest: reservation.schema_digest.clone(),
                        binding: reservation.binding.clone(),
                    },
                )
            })
            .collect();
        *self
            .event_index
            .0
            .write()
            .map_err(|_| "event index is unavailable")? = projection;
        Ok(())
    }

    pub(super) fn event_port(&self, program_instance_id: String) -> Arc<dyn EventPort> {
        Arc::new(ReservedEvents {
            program_instance_id,
            index: self.event_index.clone(),
        })
    }

    /// Reconcile the two durable stores after any execution commit or restart.
    /// The commit adapter's continuation owns the exact wait; metadata records
    /// its immutable binding and terminal wake before any worker can claim it.
    pub(super) fn reconcile_event_waits(&mut self) -> Result<(), String> {
        let mut bindings = Vec::new();
        for (instance_id, instance) in &self.instances {
            let Some(invocation) = instance.invocation.as_ref() else {
                continue;
            };
            if invocation.result.is_some() {
                continue;
            }
            let Some(committed) = self
                .execution_backend
                .load_continuation(&ProgramInstanceRef::new(instance_id.clone()))
            else {
                continue;
            };
            let continuation: Continuation = serde_json::from_value(committed.payload)
                .map_err(|error| format!("invalid committed continuation: {error}"))?;
            let Some(reference) = continuation
                .event_ref
                .as_ref()
                .and_then(EventRef::reservation)
            else {
                continue;
            };
            if continuation.program_instance_ref.as_str() != instance_id
                || continuation.program_invocation_ref.as_str() != invocation.program_invocation_id
            {
                return Err("Event continuation instance or invocation mismatch".into());
            }
            let reservation = self
                .reservations
                .get(&(reference.event_id.clone(), reference.generation))
                .ok_or("committed Event wait has no reservation")?;
            let requirement = continuation
                .air
                .event_requirements
                .iter()
                .find(|requirement| requirement.node_id == continuation.continuation_id)
                .ok_or("committed Event wait has no declaration")?;
            if reservation.program_instance_id != *instance_id
                || reservation.type_id != requirement.type_id
                || reservation.schema_digest != requirement.schema_digest
                || reservation.payload_schema != requirement.payload_schema
            {
                return Err("committed Event wait violates reservation contract".into());
            }
            let binding = EventWaitBinding {
                program_instance_id: instance_id.clone(),
                program_invocation_id: invocation.program_invocation_id.clone(),
                node_id: continuation.continuation_id,
                node_execution_id: continuation
                    .parked_node_execution_id
                    .ok_or("Event wait lacks node execution")?,
                event_ref: reference.clone(),
            };
            if reservation
                .binding
                .as_ref()
                .is_some_and(|prior| prior != &binding)
            {
                return Err("Event reservation was already consumed by a different wait".into());
            }
            if reservation.binding.is_none() {
                bindings.push(binding);
            }
        }
        if bindings.is_empty() {
            return Ok(());
        }
        for binding in &bindings {
            self.reservations
                .get_mut(&(
                    binding.event_ref.event_id.clone(),
                    binding.event_ref.generation,
                ))
                .expect("validated reservation")
                .binding = Some(binding.clone());
        }
        if let Err(error) = self.persist_runtime_state() {
            for binding in bindings {
                self.reservations
                    .get_mut(&(binding.event_ref.event_id, binding.event_ref.generation))
                    .expect("validated reservation")
                    .binding = None;
            }
            return Err(error);
        }
        Ok(())
    }

    pub(super) fn claim_reserved_event_resume(
        &mut self,
        reference: &CanonicalEventRef,
    ) -> Result<Option<PreparedResume>, String> {
        let state = self
            .reservations
            .get(&(reference.event_id.clone(), reference.generation))
            .ok_or("unknown Event reservation")?;
        if state.resume_started || state.binding.is_none() || state.status == EventStatus::Pending {
            return Ok(None);
        }
        let delivery = match state.status {
            EventStatus::Fulfilled => {
                let application = self
                    .applications
                    .values()
                    .find(|application| &application.event_ref == reference)
                    .ok_or("fulfilled Event lacks committed application")?;
                EventDelivery::Fulfilled {
                    payload: application.payload.clone(),
                }
            }
            EventStatus::Expired => EventDelivery::Expired,
            EventStatus::Cancelled => EventDelivery::Cancelled,
            EventStatus::Pending => unreachable!(),
        };
        let instance_id = state.program_instance_id.clone();
        let target = EventRef::reserved(reference.clone()).map_err(|error| error.to_string())?;
        let delivered = serde_json::to_value(delivery).map_err(|error| error.to_string())?;
        self.claim_continuation_resume(instance_id, target, delivered)
    }

    /// One lease construction path for both reservation delivery and host
    /// settlement. Only the immutable typed delivered value differs.
    pub(super) fn claim_continuation_resume(
        &mut self,
        instance_id: String,
        event_ref: EventRef,
        delivered: Value,
    ) -> Result<Option<PreparedResume>, String> {
        let expected_version = block_on(
            self.execution_backend
                .commit_port()
                .current_version(&ProgramInstanceRef::new(instance_id.clone())),
        );
        let Some(committed) = self
            .execution_backend
            .load_continuation(&ProgramInstanceRef::new(instance_id.clone()))
        else {
            return Ok(None);
        };
        let continuation: Continuation = serde_json::from_value(committed.payload.clone())
            .map_err(|error| format!("invalid committed continuation: {error}"))?;
        if continuation.event_ref.as_ref() != Some(&event_ref) {
            return Ok(None);
        }
        let instance = self.instances.get(&instance_id).ok_or("unknown_instance")?;
        let invocation = instance
            .invocation
            .as_ref()
            .ok_or("parked continuation has no invocation state")?;
        let invocation_id = continuation.program_invocation_ref.as_str().to_owned();
        if invocation.program_invocation_id != invocation_id
            || continuation.program_instance_ref.as_str() != instance_id
        {
            return Err("parked continuation identity mismatch".into());
        }
        if invocation.result.is_some() || self.active_cancellations.contains_key(&invocation_id) {
            return Ok(None);
        }
        if self.active_cancellations.len() >= MAX_ACTIVE_INVOCATIONS {
            return Err("invocation_capacity_exhausted".into());
        }
        let artifact_bytes = self
            .artifacts
            .get(&instance.artifact_digest)
            .ok_or("parked invocation artifact is unavailable")?
            .to_vec();
        let mut materials = instance
            .materials
            .clone()
            .ok_or("parked invocation admission is unavailable")?;
        if let Err(error) = self.verify_selected_materials(&artifact_bytes, &materials) {
            if error != "admission_profile_mismatch" {
                return Err(error);
            }
            self.fail_unstarted_profile_resume(
                &instance_id,
                &invocation_id,
                &event_ref,
                expected_version,
                &committed,
            )?;
            return Ok(None);
        }
        materials.admission.invocation_id.clone_from(&invocation_id);
        let cancellation = CancellationToken::new();
        if self.invocation_is_cancelled(&instance_id, &invocation_id) {
            cancellation.cancel();
        }
        self.active_cancellations
            .insert(invocation_id.clone(), cancellation.clone());
        Ok(Some(PreparedResume {
            event_ref,
            events: self.event_port(instance_id.clone()),
            invocation_request_id: invocation.request_id.clone(),
            program_instance_id: instance_id,
            invocation_id,
            delivered,
            continuation,
            committed_continuation: committed,
            expected_program_state_version: expected_version,
            artifact_bytes,
            materials,
            handlers: self.handlers.clone(),
            package_root: self.package_root.clone(),
            sandbox_registry: self.sandbox_registry.clone(),
            execution_backend: self.execution_backend.commit_port(),
            observation_sink: self.observation_sink.clone(),
            cancellation,
            #[cfg(test)]
            test_gate: self.resume_test_gate.clone(),
        }))
    }

    pub(super) fn fail_unstarted_profile_resume(
        &mut self,
        instance_id: &str,
        invocation_id: &str,
        event_ref: &EventRef,
        expected_version: u64,
        committed: &apxm_kernel::CommittedContinuation,
    ) -> Result<(), String> {
        let resume_started = match event_ref {
            EventRef::HostCapability { request_id } => self
                .host_capability_settlements
                .get(request_id)
                .is_some_and(|state| state.resume_started),
            EventRef::Reserved { reference } => self
                .reservations
                .get(&(reference.event_id.clone(), reference.generation))
                .is_some_and(|state| state.resume_started),
        };
        if resume_started {
            return Err("outcome_unknown".to_owned());
        }
        let instance = self
            .instances
            .get_mut(instance_id)
            .ok_or("unknown_instance")?;
        let invocation = instance.invocation.as_mut().ok_or("unknown_invocation")?;
        if invocation.program_invocation_id != invocation_id {
            return Err("parked continuation identity mismatch".to_owned());
        }
        if invocation.result.is_some() {
            return Ok(());
        }
        let committed_result = block_on(apxm_execution::commit_parked_admission_failure(
            self.execution_backend.commit_port().as_ref(),
            expected_version,
            committed,
        ))
        .map_err(|error| error.to_string())?;
        let result_code = match committed_result {
            ExecutionCommitResult::Committed { .. } => "admission_profile_mismatch",
            ExecutionCommitResult::CompareConflict { .. } => return Ok(()),
            ExecutionCommitResult::OutcomeUnknown { .. } => "outcome_unknown",
        };
        let request_id = invocation.request_id.clone();
        let result = RuntimeResult::Failed {
            request_id: request_id.clone(),
            code: result_code.to_owned(),
        };
        invocation.result = Some(result.clone());
        if let Some(history) = instance.invocation_history.get_mut(&request_id) {
            history.result = Some(result);
        }
        if let Err(error) = self.persist_runtime_state() {
            if let Some(instance) = self.instances.get_mut(instance_id) {
                if let Some(invocation) = instance.invocation.as_mut() {
                    invocation.result = None;
                }
                if let Some(history) = instance.invocation_history.get_mut(&request_id) {
                    history.result = None;
                }
            }
            return Err(error);
        }
        self.observation_signal.notify();
        Ok(())
    }

    pub(super) fn resume_marker(&mut self, prepared: &PreparedResume) -> Option<&mut bool> {
        match &prepared.event_ref {
            EventRef::Reserved { reference } => {
                let state = self
                    .reservations
                    .get_mut(&(reference.event_id.clone(), reference.generation))?;
                let binding = state.binding.as_ref()?;
                (binding.program_instance_id == prepared.program_instance_id
                    && binding.program_invocation_id == prepared.invocation_id
                    && Some(binding.node_execution_id.as_str())
                        == prepared.continuation.parked_node_execution_id.as_deref())
                .then_some(&mut state.resume_started)
            }
            EventRef::HostCapability { request_id } => {
                let state = self.host_capability_settlements.get_mut(request_id)?;
                (state.program_instance_id == prepared.program_instance_id)
                    .then_some(&mut state.resume_started)
            }
        }
    }

    pub(super) fn reconcile_reserved_event_resumes(&mut self) -> Result<(), String> {
        self.expire_pending_events()?;
        self.reconcile_event_waits()?;
        let mut changed = false;
        for reservation in self.reservations.values_mut() {
            let Some(binding) = reservation.binding.as_ref() else {
                continue;
            };
            if !reservation.resume_started
                && reservation.status == EventStatus::Pending
                && self.cancelled.contains_key(&binding.program_invocation_id)
            {
                reservation.status = EventStatus::Cancelled;
                changed = true;
            }
        }
        if changed {
            self.persist_runtime_state()?;
        }
        self.reconcile_started_continuation_resumes()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use apxm_kernel::event_api::EventOccurrence;
    use serde_json::json;

    struct Reserved {
        service: RuntimeService,
        instance: String,
        instance_claim: RuntimeOwnerClaim,
        reference: CanonicalEventRef,
        event_claim: RuntimeOwnerClaim,
    }

    fn reserved(service: RuntimeService) -> Reserved {
        let mut service = service;
        let instance = crate::tests::create_started(&mut service, crate::tests::event_air_bytes());
        let instance_claim = service.instances[&instance].owner_claim.clone();
        let result = service
            .reserve_event(
                "reserve.1".into(),
                instance.clone(),
                instance_claim.clone(),
                "UserInput".into(),
            )
            .unwrap();
        let RuntimeResult::EventReserved {
            event_ref: reference,
            owner_claim: event_claim,
            ..
        } = result
        else {
            panic!("{result:?}")
        };
        Reserved {
            service,
            instance,
            instance_claim,
            reference,
            event_claim,
        }
    }

    fn application(reference: &CanonicalEventRef) -> EventApplication<Value> {
        EventApplication {
            event_ref: reference.clone(),
            idempotency_key: "apply.1".into(),
            occurrence: EventOccurrence {
                occurrence_id: "occurrence.1".into(),
                source_kind: "test.source".into(),
                mapping_digest: "mapping.1".into(),
                source_record: "record.1".into(),
                payload: json!({"answer":"delivered"}),
            },
        }
    }

    impl Reserved {
        fn apply(&mut self, application: EventApplication<Value>) -> EventApplicationResult {
            match self
                .service
                .fulfill_event("fulfill.1".into(), self.event_claim.clone(), application)
                .unwrap()
            {
                RuntimeResult::EventApplied { result, .. } => result,
                other => panic!("{other:?}"),
            }
        }

        fn start(&mut self) -> String {
            let result = self.service.start_invocation(
                "start.1".into(),
                self.instance.clone(),
                self.instance_claim.clone(),
                serde_json::to_value(&self.reference).unwrap(),
            );
            let RuntimeResult::ProgramInvocationStarted {
                program_invocation_id,
                ..
            } = result
            else {
                panic!("{result:?}")
            };
            program_invocation_id
        }

        fn reopen(&mut self, path: &std::path::Path) {
            self.service = RuntimeService::in_memory();
            self.service = RuntimeService::in_memory()
                .with_runtime_state_dir(path.to_path_buf())
                .with_embedded_read_access();
            assert!(
                self.service.startup_error().is_none(),
                "{:?}",
                self.service.startup_error()
            );
            self.service.reconcile_recovery_state().unwrap();
        }

        fn complete(&mut self) -> RuntimeResult {
            let prepared = self
                .service
                .claim_next_continuation_resume()
                .unwrap()
                .expect("pending Event wake");
            assert_eq!(prepared.event_ref.reservation(), Some(&self.reference));
            assert!(
                self.service
                    .claim_next_continuation_resume()
                    .unwrap()
                    .is_none(),
                "one process lease per invocation"
            );
            assert!(self.service.begin_continuation_resume(&prepared).unwrap());
            let execution = prepared.execute();
            self.service
                .finish_continuation_resume(&prepared, execution)
        }
    }

    #[test]
    fn a_reservation_is_consumed_by_one_exact_wait_visit() {
        let mut fixture = reserved(RuntimeService::in_memory());
        let invocation = fixture.start();
        let key = (
            fixture.reference.event_id.clone(),
            fixture.reference.generation,
        );
        let state = &fixture.service.reservations[&key];
        let binding = state
            .binding
            .clone()
            .expect("the first wait commits its binding");
        let exact = EventAwait {
            node_id: binding.node_id.clone(),
            node_execution_id: binding.node_execution_id.clone(),
            program_invocation_id: invocation.clone(),
            event_ref: EventRef::reserved(fixture.reference.clone()).unwrap(),
            contract: EventWaitContract::Declared {
                type_id: state.type_id.clone(),
                schema_digest: state.schema_digest.clone(),
            },
        };
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        for fulfilled in [false, true] {
            if fulfilled {
                assert_eq!(
                    fixture.apply(application(&fixture.reference)),
                    EventApplicationResult::Fulfilled
                );
            }
            let port = fixture.service.event_port(fixture.instance.clone());
            assert_eq!(
                runtime.block_on(port.await_event(exact.clone())),
                EventOutcome::Parked
            );
            for changed_coordinate in ["node", "visit", "invocation"] {
                let mut reused = exact.clone();
                match changed_coordinate {
                    "node" => reused.node_id = "second.wait".into(),
                    "visit" => reused.node_execution_id = "later.loop.visit".into(),
                    _ => reused.program_invocation_id = "later.invocation".into(),
                }
                assert_eq!(
                    runtime.block_on(port.await_event(reused)),
                    EventOutcome::Rejected {
                        reason: EventWaitRejection::AlreadyBound,
                    },
                    "{changed_coordinate}, fulfilled={fulfilled}"
                );
            }
        }
        assert_eq!(
            fixture.service.reservations[&key].binding.as_ref(),
            Some(&binding)
        );
        assert!(matches!(fixture.service.prepare_invocation(
            "concurrent.start".into(), fixture.instance.clone(), fixture.instance_claim.clone(),
            serde_json::to_value(&fixture.reference).unwrap(),
        ), Err(RuntimeResult::Failed {code, ..}) if code == "invocation_already_started"));
        fixture.complete();
    }

    #[test]
    fn reservation_is_instance_owned_idempotent_and_compiler_typed() {
        let mut fixture = reserved(RuntimeService::in_memory());
        let replay = fixture
            .service
            .reserve_event(
                "reserve.1".into(),
                fixture.instance.clone(),
                fixture.instance_claim.clone(),
                "UserInput".into(),
            )
            .unwrap();
        assert!(
            matches!(replay, RuntimeResult::EventReserved { event_ref, owner_claim, .. } if event_ref == fixture.reference && owner_claim == fixture.event_claim)
        );
        assert!(
            matches!(fixture.service.reserve_event("reserve.1".into(), fixture.instance.clone(), fixture.instance_claim.clone(), "other".into()).unwrap(), RuntimeResult::Failed { code, .. } if code == "invalid_request")
        );
        assert!(
            matches!(fixture.service.reserve_event("reserve.2".into(), fixture.instance.clone(), fixture.instance_claim.clone(), "other".into()).unwrap(), RuntimeResult::Failed { code, .. } if code == "undeclared_event_type")
        );
        assert_eq!(
            fixture
                .service
                .reserve_event(
                    "reserve.3".into(),
                    fixture.instance.clone(),
                    RuntimeOwnerClaim::mint(),
                    "UserInput".into()
                )
                .unwrap_err(),
            ProtocolError::OwnerMismatch
        );
        assert_eq!(fixture.service.reservations.len(), 1);
    }

    #[test]
    fn terminal_replay_compares_full_occurrence_under_every_key() {
        let mut fixture = reserved(RuntimeService::in_memory());
        let original = application(&fixture.reference);
        let mut malformed = original.clone();
        malformed.occurrence.payload = json!({"answer":42});
        assert_eq!(fixture.apply(malformed), EventApplicationResult::Rejected);
        assert!(fixture.service.applications.is_empty());
        let mut wrong_generation = original.clone();
        wrong_generation.event_ref.generation += 1;
        assert_eq!(
            fixture.apply(wrong_generation),
            EventApplicationResult::Rejected
        );
        assert_eq!(
            fixture.apply(original.clone()),
            EventApplicationResult::Fulfilled
        );
        for same_key in [true, false] {
            let mut replay = original.clone();
            if !same_key {
                replay.idempotency_key = "different.key".into();
            }
            assert_eq!(
                fixture.apply(replay.clone()),
                EventApplicationResult::Fulfilled
            );
            for coordinate in 0..5 {
                let mut changed = replay.clone();
                match coordinate {
                    0 => changed.occurrence.payload = json!({"answer":"changed"}),
                    1 => changed.occurrence.occurrence_id = "occurrence.changed".into(),
                    2 => changed.occurrence.source_kind = "source.changed".into(),
                    3 => changed.occurrence.source_record = "record.changed".into(),
                    _ => changed.occurrence.mapping_digest = "mapping.changed".into(),
                }
                assert_eq!(fixture.apply(changed), EventApplicationResult::Conflict);
            }
        }
        assert_eq!(fixture.service.applications.len(), 1);
        assert_eq!(
            fixture
                .service
                .fulfill_event("wrong.owner".into(), RuntimeOwnerClaim::mint(), original)
                .unwrap_err(),
            ProtocolError::OwnerMismatch
        );
    }

    #[test]
    fn early_delivery_and_lost_ack_reopen_as_one_exact_unstarted_resume() {
        let directory = tempfile::tempdir().unwrap();
        let mut fixture = reserved(
            RuntimeService::in_memory()
                .with_runtime_state_dir(directory.path().to_path_buf())
                .with_embedded_read_access(),
        );
        let original = application(&fixture.reference);
        assert_eq!(
            fixture.apply(original.clone()),
            EventApplicationResult::Fulfilled
        );
        assert!(
            fixture.service.reservations[&(
                fixture.reference.event_id.clone(),
                fixture.reference.generation
            )]
                .binding
                .is_none()
        );
        fixture.reopen(directory.path());
        let invocation = fixture.start();
        let prepared = fixture
            .service
            .claim_next_continuation_resume()
            .unwrap()
            .unwrap();
        assert_eq!(prepared.invocation_id, invocation);
        drop(prepared); // process dies before worker start; durable wake remains unstarted
        fixture.reopen(directory.path());
        assert_eq!(fixture.apply(original), EventApplicationResult::Fulfilled);
        assert!(matches!(
            fixture.complete(),
            RuntimeResult::ProgramInvocationStarted { .. }
        ));
        fixture.reopen(directory.path());
        assert!(
            fixture
                .service
                .claim_next_continuation_resume()
                .unwrap()
                .is_none()
        );
        assert!(
            fixture.service.instances[&fixture.instance]
                .invocation
                .as_ref()
                .unwrap()
                .result
                .is_some()
        );
    }

    #[test]
    fn wait_commit_before_service_finalization_recovers_early_delivery() {
        let directory = tempfile::tempdir().unwrap();
        let mut fixture = reserved(
            RuntimeService::in_memory().with_runtime_state_dir(directory.path().to_path_buf()),
        );
        let original = application(&fixture.reference);
        assert_eq!(fixture.apply(original), EventApplicationResult::Fulfilled);
        let prepared = fixture
            .service
            .prepare_invocation(
                "start.1".into(),
                fixture.instance.clone(),
                fixture.instance_claim.clone(),
                serde_json::to_value(&fixture.reference).unwrap(),
            )
            .unwrap();
        assert!(
            fixture
                .service
                .begin_invocation(&prepared.invocation_id)
                .unwrap()
        );
        assert!(prepared.execute().is_ok());
        // No finish callback: the committed continuation must repair the metadata join.
        drop(prepared);
        fixture.reopen(directory.path());
        assert!(matches!(
            fixture.complete(),
            RuntimeResult::ProgramInvocationStarted { .. }
        ));
    }

    #[test]
    fn crossed_resume_start_fence_never_resends_after_restart() {
        let directory = tempfile::tempdir().unwrap();
        let mut fixture = reserved(
            RuntimeService::in_memory().with_runtime_state_dir(directory.path().to_path_buf()),
        );
        fixture.start();
        let original = application(&fixture.reference);
        assert_eq!(fixture.apply(original), EventApplicationResult::Fulfilled);
        let prepared = fixture
            .service
            .claim_next_continuation_resume()
            .unwrap()
            .unwrap();
        assert!(
            fixture
                .service
                .begin_continuation_resume(&prepared)
                .unwrap()
        );
        drop(prepared);
        fixture.reopen(directory.path());
        assert!(
            fixture
                .service
                .claim_next_continuation_resume()
                .unwrap()
                .is_none()
        );
        assert!(
            matches!(fixture.service.instances[&fixture.instance].invocation.as_ref().unwrap().result, Some(RuntimeResult::Failed { ref code, .. }) if code == "outcome_unknown")
        );
    }

    #[test]
    fn committed_resume_before_finish_recovers_terminal_truth_without_resend() {
        let directory = tempfile::tempdir().unwrap();
        let mut fixture = reserved(
            RuntimeService::in_memory().with_runtime_state_dir(directory.path().to_path_buf()),
        );
        fixture.start();
        let original = application(&fixture.reference);
        fixture.apply(original);
        let prepared = fixture
            .service
            .claim_next_continuation_resume()
            .unwrap()
            .unwrap();
        assert!(
            fixture
                .service
                .begin_continuation_resume(&prepared)
                .unwrap()
        );
        assert!(prepared.execute().is_ok());
        drop(prepared);
        fixture.reopen(directory.path());
        assert!(
            fixture
                .service
                .claim_next_continuation_resume()
                .unwrap()
                .is_none()
        );
        assert!(matches!(
            fixture.service.instances[&fixture.instance]
                .invocation
                .as_ref()
                .unwrap()
                .result,
            Some(RuntimeResult::ProgramInvocationStarted { .. })
        ));
    }

    #[test]
    fn wrong_instance_and_generation_cannot_bind_or_consume_reservation() {
        for wrong_instance in [false, true] {
            let mut fixture = reserved(RuntimeService::in_memory());
            let (instance, claim, reference) = if wrong_instance {
                let instance = crate::tests::create_started(
                    &mut fixture.service,
                    crate::tests::event_air_bytes(),
                );
                let claim = fixture.service.instances[&instance].owner_claim.clone();
                (instance, claim, fixture.reference.clone())
            } else {
                let mut reference = fixture.reference.clone();
                reference.generation += 1;
                (
                    fixture.instance.clone(),
                    fixture.instance_claim.clone(),
                    reference,
                )
            };
            let result = fixture.service.start_invocation(
                "wrong.target".into(),
                instance,
                claim,
                serde_json::to_value(reference).unwrap(),
            );
            assert!(matches!(result, RuntimeResult::Failed { .. }), "{result:?}");
            assert!(
                fixture.service.reservations[&(
                    fixture.reference.event_id.clone(),
                    fixture.reference.generation
                )]
                    .binding
                    .is_none()
            );
            assert!(
                fixture
                    .service
                    .claim_next_continuation_resume()
                    .unwrap()
                    .is_none()
            );
        }
    }

    #[test]
    fn expiry_and_cancel_wake_without_publishing_payload_or_fulfilling_an_effect() {
        for status in [EventStatus::Expired, EventStatus::Cancelled] {
            for early in [false, true] {
                let mut fixture = reserved(RuntimeService::in_memory());
                if !early {
                    fixture.start();
                }
                fixture
                    .service
                    .change_event_status(
                        "terminal.1".into(),
                        fixture.event_claim.clone(),
                        fixture.reference.clone(),
                        status,
                    )
                    .unwrap();
                if early {
                    fixture.start();
                }
                let original = application(&fixture.reference);
                assert_eq!(
                    fixture.apply(original),
                    if status == EventStatus::Expired {
                        EventApplicationResult::Expired
                    } else {
                        EventApplicationResult::Cancelled
                    }
                );
                let completed = fixture.complete();
                let invocation = &fixture.service.instances[&fixture.instance]
                    .invocation
                    .as_ref()
                    .unwrap()
                    .program_invocation_id;
                assert_eq!(
                    fixture
                        .service
                        .execution_backend
                        .invocation_status(invocation),
                    Some(if status == EventStatus::Expired {
                        ProgramInvocationStatus::Failed
                    } else {
                        ProgramInvocationStatus::Cancelled
                    }),
                    "{completed:?}"
                );
                assert!(fixture.service.applications.is_empty());
                assert!(matches!(fixture.service.reserve_event(
                    "after.terminal".into(), fixture.instance.clone(), fixture.instance_claim.clone(), "UserInput".into(),
                ).unwrap(), RuntimeResult::Failed {code, ..} if code == "instance_unavailable"));
            }
        }
    }

    #[test]
    fn invocation_cancellation_and_ttl_leave_a_durable_wake_not_an_orphan() {
        for explicit in [false, true] {
            let mut fixture = reserved(RuntimeService::in_memory());
            let invocation = fixture.start();
            if explicit {
                fixture.service.cancel_invocation(
                    "cancel.1".into(),
                    fixture.instance_claim.clone(),
                    invocation,
                );
            } else {
                fixture
                    .service
                    .reservations
                    .get_mut(&(
                        fixture.reference.event_id.clone(),
                        fixture.reference.generation,
                    ))
                    .unwrap()
                    .state_entry
                    .expires_at = Instant::now();
                fixture.service.cleanup_expired().unwrap();
            }
            fixture.complete();
            let invocation = &fixture.service.instances[&fixture.instance]
                .invocation
                .as_ref()
                .unwrap()
                .program_invocation_id;
            assert_eq!(
                fixture
                    .service
                    .execution_backend
                    .invocation_status(invocation),
                Some(if explicit {
                    ProgramInvocationStatus::Cancelled
                } else {
                    ProgramInvocationStatus::Failed
                })
            );
            assert_eq!(
                fixture.service.reservations.len(),
                1,
                "exact replay identity survives while its instance remains"
            );
        }
    }

    #[test]
    fn process_backpressure_retains_unstarted_delivery() {
        let mut fixture = reserved(RuntimeService::in_memory());
        fixture.start();
        let original = application(&fixture.reference);
        fixture.apply(original);
        for index in 0..MAX_ACTIVE_INVOCATIONS {
            fixture
                .service
                .active_cancellations
                .insert(format!("busy.{index}"), CancellationToken::new());
        }
        assert!(
            fixture
                .service
                .claim_next_continuation_resume()
                .unwrap()
                .is_none()
        );
        assert!(
            !fixture.service.reservations[&(
                fixture.reference.event_id.clone(),
                fixture.reference.generation
            )]
                .resume_started
        );
        fixture.service.active_cancellations.clear();
        assert!(matches!(
            fixture.complete(),
            RuntimeResult::ProgramInvocationStarted { .. }
        ));
    }
}
