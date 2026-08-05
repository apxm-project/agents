use apxm_kernel::{INVOCATION_ADMISSION_SCHEMA, InvocationAdmission, InvocationAdmissionError};

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

    assert_eq!(
        admission.validate(),
        Err(InvocationAdmissionError::ArtifactProvenanceMismatch)
    );
}

#[test]
fn invocation_admission_rejects_release_port_and_ceiling_drift() {
    let admission = valid();
    assert_eq!(
        admission.verify_against(&digest('d'), &digest('b'), &digest('c')),
        Err(InvocationAdmissionError::ReleaseMismatch)
    );
    assert_eq!(
        admission.verify_against(&digest('a'), &digest('d'), &digest('c')),
        Err(InvocationAdmissionError::PortBindingsMismatch)
    );
    assert_eq!(
        admission.verify_against(&digest('a'), &digest('b'), &digest('d')),
        Err(InvocationAdmissionError::ResourceCeilingMismatch)
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
