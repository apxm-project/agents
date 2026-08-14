use apxm_kernel::{
    AdmittedConfinement, INVOCATION_ADMISSION_SCHEMA, InvocationAdmission,
    InvocationAdmissionError, ResourceCeilings, digest_serializable, minimal_port_bindings,
    verify_invocation_admission,
};

fn digest(byte: char) -> String {
    format!("sha256:{}", byte.to_string().repeat(64))
}

fn valid() -> InvocationAdmission {
    InvocationAdmission {
        schema_version: INVOCATION_ADMISSION_SCHEMA.into(),
        invocation_id: "invocation.1".into(),
        artifact_digest: digest('a'),
        release_digest: digest('a'),
        port_bindings_digest: digest('b'),
        resource_ceiling_digest: digest('c'),
        provenance_digest: digest('a'),
    }
}

#[test]
fn invocation_admission_validates_exact_contract_and_host_digests() {
    let admission = valid();
    admission
        .verify_against(&digest('a'), &digest('b'), &digest('c'))
        .expect("exact invocation admission");
}

#[test]
fn invocation_admission_rejects_provenance_drift_before_runtime() {
    let mut admission = valid();
    admission.provenance_digest = digest('d');

    let port_bindings = minimal_port_bindings();
    let resource_ceilings = ResourceCeilings {
        max_wall_ms: 60_000,
        max_memory_bytes: 64 * 1024 * 1024,
        max_effect_bytes: 1024 * 1024,
    };
    admission.artifact_digest =
        "sha256:c7c5c1d70c5dec4416ab6158afd0b223ef40c29b1dc1f97ed9428b94d4cadb1c".into();
    admission.release_digest =
        "sha256:a4d451ec23463726f72c43d64c710968f6b602cd653b4de8adee1b556240a829".into();
    admission.port_bindings_digest = digest_serializable(&port_bindings).expect("port digest");
    admission.resource_ceiling_digest =
        digest_serializable(&resource_ceilings).expect("ceiling digest");

    let error = verify_invocation_admission(
        &admission,
        b"artifact",
        b"release",
        b"provenance",
        &[],
        &port_bindings,
        resource_ceilings,
        &AdmittedConfinement {
            confinement_type: "NATIVE-SANDBOX".into(),
            sandbox_digest: digest('d'),
            policy_digest: digest('e'),
        },
    );
    let error = error.expect_err("provenance drift");
    assert_eq!(
        error,
        InvocationAdmissionError::ProvenanceMismatch {
            expected: "sha256:96d815328a42cb4ef89d5e0b7a1df6be43b484832c83a7b4596d8402c7c0b12b"
                .into(),
            actual: digest('d'),
        }
    );
}

#[test]
fn invocation_admission_rejects_release_port_and_ceiling_drift() {
    let admission = valid();
    assert_eq!(
        admission.verify_against(&digest('d'), &digest('b'), &digest('c')),
        Err(InvocationAdmissionError::ReleaseMismatch {
            expected: digest('d'),
            actual: digest('a'),
        })
    );
    assert_eq!(
        admission.verify_against(&digest('a'), &digest('d'), &digest('c')),
        Err(InvocationAdmissionError::PortBindingsDigestMismatch {
            expected: digest('d'),
            actual: digest('b'),
        })
    );
    assert_eq!(
        admission.verify_against(&digest('a'), &digest('b'), &digest('d')),
        Err(InvocationAdmissionError::ResourceCeilingDigestMismatch {
            expected: digest('d'),
            actual: digest('c'),
        })
    );
}

#[test]
fn invocation_admission_rejects_malformed_shape_and_unknown_fields() {
    let mut value = serde_json::to_value(valid()).expect("admission json");
    value["provenance_digest"] = serde_json::Value::String("not-a-digest".into());
    let decoded: InvocationAdmission = serde_json::from_value(value).expect("wire shape");
    assert_eq!(
        decoded.validate(),
        Err(InvocationAdmissionError::MalformedDigest(
            "provenance_digest"
        ))
    );

    let mut value = serde_json::to_value(valid()).expect("admission json");
    value["ambient_credentials"] = serde_json::Value::Bool(true);
    assert!(serde_json::from_value::<InvocationAdmission>(value).is_err());
}

/// A program that states a Capability requirement is not admitted through a
/// Runtime Profile that binds no Capability Port. This is the Capability mirror
/// of the model-target check: `artifact_semantic_requirements` used to appear
/// nowhere under the runtime at all, so a requirement crossed into execution
/// with nothing at the boundary asking whether it could be satisfied.
#[test]
fn a_capability_requirement_is_refused_when_no_capability_port_is_admitted() {
    let air: apxm_program::AirModule = serde_json::from_value(serde_json::json!({
        "schema_version": "apxm.air",
        "semantic_operations": [{
            "node_id": "n.cap",
            "op": "capability.invoke",
            "parent_region_id": "r.fn",
            "execution_order": 0,
            "operands": [
                {"slot": "capability_ref", "value_id": "read", "type_ref": "CapabilityRef"},
                {"slot": "arguments", "value_id": "value.args", "type_ref": "Arguments"}
            ]
        }],
        "structural_ir": [{"region_id": "r.fn", "kind": "function", "execution_order": 0}],
        "context_flow": [],
        "source_map": {
            "schema_version": "apxm.source-map",
            "source_language": "python",
            "node_spans": [],
            "region_annotations": []
        }
    }))
    .expect("probe AIR");
    let requirements = apxm_program::air_semantic_requirements(&air);
    assert_eq!(requirements.len(), 1);

    // `minimal_port_bindings` admits execution_commit and confinement only.
    let port_bindings = minimal_port_bindings();
    let (exact, _) = apxm_kernel::resolve_exact_bindings(&port_bindings).expect("exact bindings");
    let error = apxm_kernel::reconcile_artifact_requirements(&requirements, &exact)
        .expect_err("a Capability requirement needs an admitted Capability Port");
    assert_eq!(
        error,
        apxm_kernel::RequirementReconciliationError::UnadmittedSlot {
            typed_port_slot: "read".into(),
            slot: apxm_kernel::PortSlot::Capability,
        }
    );
}
