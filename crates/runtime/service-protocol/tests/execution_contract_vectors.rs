//! Published schema vectors for the additive execution observation/read surface.

use std::fs;
use std::path::{Path, PathBuf};

use jsonschema::JSONSchema;
use serde_json::Value;

fn contracts_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../contracts")
}

fn load(path: impl AsRef<Path>) -> Value {
    let path = path.as_ref();
    serde_json::from_str(
        &fs::read_to_string(path)
            .unwrap_or_else(|error| panic!("read {}: {error}", path.display())),
    )
    .unwrap_or_else(|error| panic!("parse {}: {error}", path.display()))
}

fn schema(name: &str) -> JSONSchema {
    let root = contracts_root();
    fn rebase(mut document: Value) -> (String, Value) {
        let id = document["$id"].as_str().expect("schema id").to_owned();
        let scoped = format!("json-schema:///{id}");
        document["$id"] = Value::String(scoped.clone());
        (scoped, document)
    }
    fn anchor_self_refs(document: &mut Value, scope: &str) {
        match document {
            Value::Object(map) => {
                if let Some(Value::String(reference)) = map.get_mut("$ref") {
                    if let Some(fragment) = reference.strip_prefix('#') {
                        *reference = format!("{scope}#{fragment}");
                    }
                }
                for value in map.values_mut() {
                    anchor_self_refs(value, scope);
                }
            }
            Value::Array(values) => {
                for value in values {
                    anchor_self_refs(value, scope);
                }
            }
            _ => {}
        }
    }
    let (_, mut document) = rebase(load(root.join("schemas").join(name)));
    let document_scope = document["$id"].as_str().expect("schema id").to_owned();
    anchor_self_refs(&mut document, &document_scope);
    let mut options = JSONSchema::options();
    for sibling in [
        "apxm.execution-observation.v1.json",
        "apxm.node-execution-inspection.v1.json",
        "apxm.session-output-ref.v1.json",
    ] {
        let (id, mut sibling_document) = rebase(load(root.join("schemas").join(sibling)));
        anchor_self_refs(&mut sibling_document, &id);
        options.with_document(id, sibling_document);
    }
    options
        .compile(&document)
        .unwrap_or_else(|error| panic!("compile {name}: {error}"))
}

#[test]
fn published_execution_contract_vectors_match_their_schemas() {
    for name in [
        "apxm.execution-observation.v1.json",
        "apxm.node-execution-inspection.v1.json",
        "apxm.session-output-ref.v1.json",
        "apxm.execution-read.v1.json",
    ] {
        let validator = schema(name);
        let vectors = load(contracts_root().join("vectors").join(name));
        let vectors = vectors.as_array().expect("vector array");
        assert!(
            vectors
                .iter()
                .any(|vector| vector["expected_valid"] == true)
        );
        assert!(
            vectors
                .iter()
                .any(|vector| vector["expected_valid"] == false)
        );
        for vector in vectors {
            let valid = validator.is_valid(&vector["input"]);
            assert_eq!(
                valid, vector["expected_valid"],
                "{name}: vector {}",
                vector["name"]
            );
        }
    }
}
