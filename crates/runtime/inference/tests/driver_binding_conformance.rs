//! The graph-hint capability digest a driver binding carries as evidence.
//!
//! A capability surface that changed between admission and dispatch is caught
//! before a send rather than explained after one.

use apxm_inference::{InferenceDriverBinding, InferenceTargetCommitment, ResolvedModelBinding};

const DIGEST_A: &str = "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const DIGEST_B: &str = "sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
const DIGEST_C: &str = "sha256:cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc";
const DIGEST_D: &str = "sha256:dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd";
const ADMITTED: &str = "sha256:5555555555555555555555555555555555555555555555555555555555555555";
const WIDENED: &str = "sha256:6666666666666666666666666666666666666666666666666666666666666666";

fn binding() -> InferenceDriverBinding {
    let resolved = ResolvedModelBinding::from_target_commitment(
        InferenceTargetCommitment::commit(
            "model.alpha",
            DIGEST_B,
            "deploy.alpha",
            DIGEST_A,
            DIGEST_C,
            DIGEST_D,
            0,
        )
        .expect("target commitment"),
    );
    InferenceDriverBinding::from_resolved("driver.exact", "profile.exact", &resolved)
        .expect("exact driver binding")
}

#[test]
fn graph_hint_capability_drift_fails_before_send() {
    let admitted = binding()
        .with_graph_hint_capability_digest(ADMITTED)
        .expect("a digest is accepted");
    assert_eq!(
        admitted.graph_hint_capability_digest.as_deref(),
        Some(ADMITTED)
    );
    admitted
        .authorize_graph_hint_capabilities(Some(ADMITTED))
        .expect("the admitted capability surface authorizes");
    assert!(
        admitted
            .authorize_graph_hint_capabilities(Some(WIDENED))
            .is_err(),
        "a widened capability surface must not authorize in place",
    );
    assert!(
        admitted.authorize_graph_hint_capabilities(None).is_err(),
        "a vanished capability claim is drift too",
    );
}

#[test]
fn a_binding_without_a_capability_claim_accepts_none() {
    let binding = binding();
    assert!(binding.graph_hint_capability_digest.is_none());
    binding
        .authorize_graph_hint_capabilities(None)
        .expect("no claim authorizes no claim");
    assert!(
        binding
            .authorize_graph_hint_capabilities(Some(ADMITTED))
            .is_err(),
        "an unadmitted capability claim must not appear at dispatch",
    );
}

#[test]
fn a_capability_digest_must_be_a_digest() {
    assert!(
        binding()
            .with_graph_hint_capability_digest("capabilities-v2")
            .is_err()
    );
    let mut invalid = binding();
    invalid.graph_hint_capability_digest = Some("capabilities-v2".to_string());
    assert!(
        invalid.validate_shape().is_err(),
        "a malformed capability digest must not pass shape validation",
    );
}

/// The evidence field is optional on the wire, so a binding admitted before
/// the capability surface existed still decodes.
#[test]
fn the_capability_digest_is_optional_on_the_wire() {
    let mut document = serde_json::to_value(binding()).expect("serialize");
    assert!(
        document.get("graph_hint_capability_digest").is_none(),
        "an absent claim must not serialize as null",
    );
    document["graph_hint_capability_digest"] = serde_json::json!(ADMITTED);
    let decoded: InferenceDriverBinding =
        serde_json::from_value(document).expect("decode with the claim present");
    assert_eq!(
        decoded.graph_hint_capability_digest.as_deref(),
        Some(ADMITTED)
    );
}
