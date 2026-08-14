//! Conformance: every published conformance vector is held against the
//! published schema it claims to exercise, and every published Port Contract is
//! held against the schemas it pins by content address.
//!
//! Before this file existed, twenty vector files under `contracts/vectors/`
//! were read by no code in any language, and five of the six published Port
//! Contracts had no reader at all — exactly the condition
//! `skill_conformance.rs` was written to end for the skill schemas. A contract
//! nobody reads is worse than no contract: it looks normative, it is cited, and
//! it drifts silently because nothing can disagree with it.
//!
//! Three things are enforced here.
//!
//! 1. Every vector's `expected_valid` verdict is the verdict its published
//!    schema actually produces. Editing either side alone fails.
//! 2. Every Port Contract's `request`/`result`/`failure` schema digest is the
//!    SHA-256 of the published schema file's exact bytes. Editing any schema
//!    that crosses a port without republishing that port's digest fails.
//! 3. The census: every file under `contracts/vectors/` is claimed by a named
//!    runner. A vector added without a reader fails here rather than joining
//!    the twenty.

mod common;

use std::collections::{BTreeMap, BTreeSet};

use common::{
    Vector, compile_schema_with, contract_file_digest, contract_file_exists, load_contract,
    load_vectors, published_contract_files, published_vector_files,
};
use serde_json::Value;

/// The immutable external-owner snapshot every document schema that references
/// shared identifier and digest primitives resolves against.
const CONTRACT_COMMON: &str = "apxm.contract-common.v1.json";

/// One published document schema and the vectors that exercise it, with
/// whatever it references: external owner snapshots first, then sibling
/// schemas this repository publishes itself.
struct SchemaUnderTest {
    /// The schema id, which is also its file stem and its vector file's stem.
    id: &'static str,
    snapshots: &'static [&'static str],
    siblings: &'static [&'static str],
}

const SCHEMAS_UNDER_TEST: &[SchemaUnderTest] = &[
    SchemaUnderTest {
        id: "apxm.capability-invocation",
        snapshots: &[CONTRACT_COMMON],
        siblings: &[],
    },
    SchemaUnderTest {
        id: "apxm.capability-outcome",
        snapshots: &[],
        siblings: &[],
    },
    SchemaUnderTest {
        id: "apxm.committed-native-model-usage",
        snapshots: &[CONTRACT_COMMON],
        siblings: &["schemas/apxm.runtime-evidence.json"],
    },
    SchemaUnderTest {
        id: "apxm.diagnostic-correlation",
        snapshots: &[CONTRACT_COMMON],
        siblings: &[],
    },
    SchemaUnderTest {
        id: "apxm.durable-event-outcome",
        snapshots: &[CONTRACT_COMMON],
        siblings: &[],
    },
    SchemaUnderTest {
        id: "apxm.durable-event-request",
        snapshots: &[CONTRACT_COMMON],
        siblings: &[],
    },
    SchemaUnderTest {
        id: "apxm.execution-admission",
        snapshots: &[CONTRACT_COMMON],
        siblings: &[],
    },
    SchemaUnderTest {
        id: "apxm.external-agent-outcome",
        snapshots: &[CONTRACT_COMMON],
        siblings: &["schemas/apxm.external-agent-evidence.json"],
    },
    SchemaUnderTest {
        id: "apxm.external-agent-request",
        snapshots: &[CONTRACT_COMMON],
        siblings: &[],
    },
    SchemaUnderTest {
        id: "apxm.inference-credential-lease",
        snapshots: &[CONTRACT_COMMON],
        siblings: &[],
    },
    SchemaUnderTest {
        id: "apxm.inference-usage-lineage",
        snapshots: &[CONTRACT_COMMON],
        siblings: &[],
    },
    SchemaUnderTest {
        id: "apxm.invocation-admission",
        snapshots: &[],
        siblings: &[],
    },
    SchemaUnderTest {
        id: "apxm.model-binding",
        snapshots: &[CONTRACT_COMMON],
        siblings: &[],
    },
    SchemaUnderTest {
        id: "apxm.model-inference-outcome",
        snapshots: &[CONTRACT_COMMON],
        siblings: &[],
    },
    SchemaUnderTest {
        id: "apxm.model-inference-request",
        snapshots: &[CONTRACT_COMMON],
        siblings: &[],
    },
    SchemaUnderTest {
        id: "apxm.model-target",
        snapshots: &[CONTRACT_COMMON],
        siblings: &[],
    },
    SchemaUnderTest {
        id: "apxm.port-contract",
        snapshots: &[],
        siblings: &[],
    },
    SchemaUnderTest {
        id: "apxm.program-composition-outcome",
        snapshots: &[CONTRACT_COMMON],
        siblings: &[],
    },
    SchemaUnderTest {
        id: "apxm.program-composition-request",
        snapshots: &[CONTRACT_COMMON],
        siblings: &[],
    },
];

/// Vector files whose reader lives elsewhere, with where to find it. The census
/// below is complete only because these are accounted for by name.
const VECTORS_READ_ELSEWHERE: &[(&str, &str)] = &[
    ("apxm.agent.json", "crates/tools/cli/src/commands/agent.rs"),
    ("apxm.air.json", "tests/semantic_conformance.rs"),
    (
        "apxm.executable-artifact.json",
        "tests/artifact_conformance.rs",
    ),
    ("apxm.execution-commit.json", "tests/runtime_conformance.rs"),
    (
        "apxm.external-agent-evidence.json",
        "tests/runtime_conformance.rs",
    ),
    (
        "apxm.external-agent-session.json",
        "tests/runtime_conformance.rs",
    ),
    (
        "apxm.frontend-conformance.json",
        "crates/tools/cli/src/frontend/codegen_frontend_conformance.rs",
    ),
    ("apxm.frontend-graph.json", "tests/semantic_conformance.rs"),
    (
        "apxm.frontend-surface.json",
        "tools/scripts/check_frontend_surface.py",
    ),
    (
        "apxm.handler-manifest.json",
        "tests/handler_manifest_conformance.rs",
    ),
    (
        "apxm.inference-driver-binding.json",
        "tests/graph_hint_conformance.rs",
    ),
    (
        "apxm.inference-graph-hints.json",
        "tests/graph_hint_conformance.rs",
    ),
    (
        "apxm.package-local-skill.json",
        "tests/skill_conformance.rs",
    ),
    ("apxm.runtime-evidence.json", "tests/runtime_conformance.rs"),
    (
        "apxm.skill-discovery-root.json",
        "tests/skill_conformance.rs",
    ),
    ("apxm.skill-package.json", "tests/skill_conformance.rs"),
    ("apxm.source-map.json", "tests/semantic_conformance.rs"),
];

fn schema_file(id: &str) -> String {
    format!("schemas/{id}.json")
}

fn vector_file(id: &str) -> String {
    format!("{id}.json")
}

/// Rejections no JSON Schema can express: a digest that does not reproduce its
/// own preimage, a field that must agree with another field, an encoding that
/// must be canonical. The published schema accepts these documents because they
/// are structurally well-formed; only the owning Rust verifier refuses them, and
/// `crates/runtime/execution/tests/contract_document_conformance.rs` is where
/// that verdict is held.
///
/// This list is asserted exactly, not merely tolerated. A schema that tightens
/// enough to reject one of these fails here and must drop its entry; a vector
/// that stops being structurally valid fails here too.
const DECIDED_ONLY_BY_THE_RUST_VERIFIER: &[(&str, &str)] = &[
    (
        "apxm.capability-invocation",
        "mutated-effect-id-is-rejected",
    ),
    (
        "apxm.capability-invocation",
        "mutated-request-digest-is-rejected",
    ),
    (
        "apxm.capability-invocation",
        "non-canonical-argument-json-is-rejected",
    ),
    (
        "apxm.committed-native-model-usage",
        "rejects-substituted-source-contract-digest",
    ),
    (
        "apxm.committed-native-model-usage",
        "rejects-replayed-measurement-identity-under-another-commit",
    ),
    (
        "apxm.diagnostic-correlation",
        "reject-correlation-digest-tamper",
    ),
    (
        "apxm.diagnostic-correlation",
        "reject-inconsistent-diagnostic-target-commitment",
    ),
    (
        "apxm.inference-credential-lease",
        "reject-lease-digest-mutation",
    ),
    (
        "apxm.inference-usage-lineage",
        "reject-lineage-digest-mutation",
    ),
    (
        "apxm.inference-usage-lineage",
        "reject-partial-evidence-binding",
    ),
];

/// The verdict the published schema actually produces for every vector that
/// claims to exercise it must be the verdict the vector claims, except where
/// the rejection is semantic and recorded above.
#[test]
fn every_published_vector_matches_its_published_schema() {
    let semantic: BTreeSet<(String, String)> = DECIDED_ONLY_BY_THE_RUST_VERIFIER
        .iter()
        .map(|(id, name)| ((*id).to_owned(), (*name).to_owned()))
        .collect();
    let mut seen: BTreeSet<(String, String)> = BTreeSet::new();

    for SchemaUnderTest {
        id,
        snapshots,
        siblings,
    } in SCHEMAS_UNDER_TEST
    {
        let schema = compile_schema_with(&schema_file(id), snapshots, siblings);
        let vectors = load_vectors(&vector_file(id));
        assert!(
            !vectors.is_empty(),
            "{id}: a vector file that exercises nothing enforces nothing"
        );
        assert!(
            vectors.iter().any(|vector| vector.expected_valid)
                && vectors.iter().any(|vector| !vector.expected_valid),
            "{id}: vectors must pin both sides of the boundary — a file of only \
             accepting or only rejecting cases cannot locate it"
        );
        for Vector {
            name,
            input,
            expected_valid,
        } in vectors
        {
            let admitted = schema.is_valid(&input);
            if admitted && !expected_valid {
                let entry = ((*id).to_owned(), name.clone());
                assert!(
                    semantic.contains(&entry),
                    "{id}: vector '{name}' expects rejection but the published schema \
                     admits it, and it is not recorded as a rejection only the Rust \
                     verifier can make"
                );
                seen.insert(entry);
                continue;
            }
            assert_eq!(
                admitted, expected_valid,
                "{id}: vector '{name}' expected valid={expected_valid} but the \
                 published schema returned {admitted}",
            );
        }
    }

    assert_eq!(
        seen, semantic,
        "a vector recorded as decidable only by the Rust verifier is now decided by \
         the published schema; drop its entry rather than leaving a stale exemption"
    );
}

/// A document schema either identifies itself — `schema_version` pinned to its
/// own `$id`, so a stored document can be checked against the contract it claims
/// — or it is a payload crossing a Port Contract that already pins its identity,
/// in which case it carries no self-identifying field at all. Both are closed
/// against fields they do not describe. This test states which schema is which,
/// so a payload that grows a `schema_version`, or a stored document that loses
/// one, is a decision someone has to make rather than a diff nobody notices.
#[test]
fn every_document_schema_either_identifies_itself_or_is_pinned_by_a_port() {
    /// Payload schemas whose identity is pinned by the Port Contract that names
    /// them as its request, result, or failure schema.
    const PINNED_BY_A_PORT_CONTRACT: &[&str] = &[
        "apxm.capability-outcome",
        "apxm.durable-event-outcome",
        "apxm.durable-event-request",
        "apxm.external-agent-outcome",
        "apxm.external-agent-request",
        "apxm.model-inference-outcome",
        "apxm.model-inference-request",
        "apxm.program-composition-outcome",
        "apxm.program-composition-request",
    ];

    for SchemaUnderTest { id, .. } in SCHEMAS_UNDER_TEST {
        let schema = load_contract(&schema_file(id));
        assert_eq!(
            schema["$id"], **id,
            "{id}: a schema addressed by an id it does not declare cannot be resolved"
        );

        let self_identifying = &schema["properties"]["schema_version"]["const"];
        if PINNED_BY_A_PORT_CONTRACT.contains(id) {
            assert!(
                self_identifying.is_null(),
                "{id}: a port payload states no schema_version; the Port Contract \
                 pins its identity"
            );
            continue;
        }
        assert_eq!(
            self_identifying,
            &Value::String((*id).to_owned()),
            "{id}: schema_version must pin the schema's own id"
        );
        assert_eq!(
            schema["additionalProperties"],
            Value::Bool(false),
            "{id}: an open document schema admits fields it does not describe"
        );
    }

    let pinned: BTreeSet<&str> = PINNED_BY_A_PORT_CONTRACT.iter().copied().collect();
    let mut named_by_a_port: BTreeSet<String> = BTreeSet::new();
    for file in port_contract_files() {
        let document = load_contract(&format!("port-contracts/{file}"));
        for slot in ["request_schema", "result_schema", "failure_schema"] {
            named_by_a_port.insert(
                document[slot]["schema_id"]
                    .as_str()
                    .expect("a schema id")
                    .to_owned(),
            );
        }
    }
    for id in pinned {
        assert!(
            named_by_a_port.contains(id),
            "{id} claims a Port Contract pins its identity, but no published Port \
             Contract names it"
        );
    }
}

// ---------------------------------------------------------------------------
// Port Contracts
// ---------------------------------------------------------------------------

/// `apxm.execution-commit` is named as the request, result, and failure schema
/// of the execution-commit Port Contract, but no `contracts/schemas` file
/// publishes it — its verifier (`verify_execution_commit_json`) and its vectors
/// are the only published form. That gap is recorded here by name rather than
/// skipped silently: publishing the schema, or a second schema going missing,
/// both fail the digest test below.
const UNPUBLISHED_PORT_SCHEMAS: &[&str] = &["apxm.execution-commit"];

fn port_contract_files() -> Vec<String> {
    published_contract_files("port-contracts")
}

#[test]
fn every_published_port_contract_validates_against_the_port_contract_schema() {
    let schema = compile_schema_with(&schema_file("apxm.port-contract"), &[], &[]);
    let files = port_contract_files();
    assert!(
        !files.is_empty(),
        "contracts/port-contracts must publish at least one Port Contract"
    );
    for file in files {
        let document = load_contract(&format!("port-contracts/{file}"));
        assert!(
            schema.is_valid(&document),
            "{file} does not satisfy the published apxm.port-contract schema: {:?}",
            compile_schema_with(&schema_file("apxm.port-contract"), &[], &[])
                .validate(&document)
                .err()
                .map(|errors| errors.map(|e| e.to_string()).collect::<Vec<_>>())
        );
    }
}

/// A Port Contract's filename, its `port_contract_id`, and the slot id the
/// runtime admits bindings for are the same string. A file whose name and id
/// disagree is addressable two ways and pinnable neither.
#[test]
fn every_port_contract_id_matches_the_file_that_publishes_it() {
    for file in port_contract_files() {
        let document = load_contract(&format!("port-contracts/{file}"));
        let expected = file
            .strip_suffix(".port-contract.json")
            .unwrap_or_else(|| panic!("{file}: port contracts are named <id>.port-contract.json"));
        assert_eq!(
            document["port_contract_id"], *expected,
            "{file}: filename and port_contract_id disagree"
        );
    }
}

/// The load-bearing test: each Port Contract pins the schemas that cross it by
/// the SHA-256 of their published bytes. Editing a schema without republishing
/// every Port Contract that pins it fails here — which is the whole reason a
/// content address is published rather than a version string.
#[test]
fn every_port_contract_digest_pins_the_published_schema_bytes() {
    let mut unpublished = BTreeSet::new();
    let mut checked = 0usize;

    for file in port_contract_files() {
        let document = load_contract(&format!("port-contracts/{file}"));
        for slot in ["request_schema", "result_schema", "failure_schema"] {
            let reference = &document[slot];
            let schema_id = reference["schema_id"]
                .as_str()
                .unwrap_or_else(|| panic!("{file}: {slot} names no schema_id"));
            let pinned = reference["digest"]
                .as_str()
                .unwrap_or_else(|| panic!("{file}: {slot} pins no digest"));
            let relative = schema_file(schema_id);
            if !contract_file_exists(&relative) {
                unpublished.insert(schema_id.to_owned());
                continue;
            }
            assert_eq!(
                contract_file_digest(&relative),
                pinned,
                "{file}: {slot} pins {schema_id} at a digest that is not the \
                 published schema's bytes — either the schema was edited without \
                 republishing this Port Contract, or the digest was written by hand"
            );
            checked += 1;
        }
    }

    assert!(
        checked > 0,
        "no Port Contract pinned a published schema, so nothing was verified"
    );
    let expected: BTreeSet<String> = UNPUBLISHED_PORT_SCHEMAS
        .iter()
        .map(|id| (*id).to_owned())
        .collect();
    assert_eq!(
        unpublished, expected,
        "the set of Port Contract schemas with no published file changed; a \
         schema that gained a file must be removed from UNPUBLISHED_PORT_SCHEMAS, \
         and one that lost its file is a break, not a bookkeeping update"
    );
}

/// Every schema a Port Contract pins is also exercised by vectors, so the
/// digest above pins bytes that something actually holds a verdict against.
#[test]
fn every_port_contract_schema_is_exercised_by_vectors() {
    let published: BTreeSet<String> = published_vector_files().into_iter().collect();
    for file in port_contract_files() {
        let document = load_contract(&format!("port-contracts/{file}"));
        for slot in ["request_schema", "result_schema", "failure_schema"] {
            let schema_id = document[slot]["schema_id"]
                .as_str()
                .unwrap_or_else(|| panic!("{file}: {slot} names no schema_id"));
            assert!(
                published.contains(&vector_file(schema_id)),
                "{file}: {slot} pins {schema_id}, which publishes no vectors"
            );
        }
    }
}

// ---------------------------------------------------------------------------
// apxm.model-context-envelope
// ---------------------------------------------------------------------------

/// `apxm.model-context-envelope` cannot be compiled from its published bytes:
/// it references `apxm.runtime-common.v1`, and no such snapshot is checked in
/// under `tests/fixtures/contracts/`. Its closed Rust carrier is therefore the
/// only executable form of the contract, and it is what the vectors are held
/// against — a frame that omits its immutable digest is refused at decode.
///
/// The dangling reference is asserted rather than worked around: if the missing
/// snapshot is ever supplied, this test fails and the envelope joins the
/// schema-driven table above where it belongs.
#[test]
fn model_context_envelope_vectors_match_the_closed_rust_carrier() {
    use apxm_core::types::context_contracts::{
        MODEL_CONTEXT_ENVELOPE_SCHEMA_VERSION, ModelContextEnvelope,
    };

    let schema = load_contract(&schema_file("apxm.model-context-envelope"));
    let text = schema.to_string();
    assert!(
        text.contains("apxm.runtime-common.v1"),
        "the envelope schema no longer references an unpublished snapshot; compile \
         it and move it into SCHEMAS_UNDER_TEST"
    );
    assert!(
        !contract_file_exists("schemas/apxm.runtime-common.v1.json"),
        "apxm.runtime-common.v1 is now published; compile the envelope schema and \
         move it into SCHEMAS_UNDER_TEST"
    );

    for Vector {
        name,
        input,
        expected_valid,
    } in load_vectors(&vector_file("apxm.model-context-envelope"))
    {
        let decoded = serde_json::from_value::<ModelContextEnvelope>(input.clone());
        assert_eq!(
            decoded.is_ok(),
            expected_valid,
            "apxm.model-context-envelope: vector '{name}' expected \
             valid={expected_valid} but the closed Rust carrier returned {:?}",
            decoded.err().map(|e| e.to_string())
        );
        if let Ok(envelope) = decoded {
            assert_eq!(
                envelope.schema_version, MODEL_CONTEXT_ENVELOPE_SCHEMA_VERSION,
                "apxm.model-context-envelope: vector '{name}' decoded under a \
                 schema_version the carrier does not own"
            );
            let reencoded = serde_json::to_value(&envelope).expect("re-encode");
            assert_eq!(
                reencoded, input,
                "apxm.model-context-envelope: vector '{name}' does not survive a \
                 decode/encode round trip, so the carrier silently drops or adds a field"
            );
        }
    }
}

// ---------------------------------------------------------------------------
// Census
// ---------------------------------------------------------------------------

/// No vector file may exist without a reader. This is the test that keeps the
/// original defect from recurring a twenty-first time: adding a vector file
/// without pointing a runner at it fails here.
#[test]
fn every_published_vector_file_is_claimed_by_a_reader() {
    const THIS_FILE: &str = "tests/contract_vector_conformance.rs";

    let mut claimed: BTreeMap<String, &str> = SCHEMAS_UNDER_TEST
        .iter()
        .map(|schema| (vector_file(schema.id), THIS_FILE))
        .collect();
    claimed.insert(vector_file("apxm.model-context-envelope"), THIS_FILE);
    for (file, reader) in VECTORS_READ_ELSEWHERE {
        assert!(
            claimed.insert((*file).to_owned(), reader).is_none(),
            "{file} is claimed twice"
        );
    }

    let published: BTreeSet<String> = published_vector_files().into_iter().collect();
    let unread: Vec<&String> = published
        .iter()
        .filter(|file| !claimed.contains_key(*file))
        .collect();
    assert!(
        unread.is_empty(),
        "these vector files are read by no code in any language — build a reader \
         or delete them: {unread:?}"
    );

    let stale: Vec<&String> = claimed
        .keys()
        .filter(|file| !published.contains(*file))
        .collect();
    assert!(
        stale.is_empty(),
        "these readers claim vector files that no longer exist: {stale:?}"
    );
}
