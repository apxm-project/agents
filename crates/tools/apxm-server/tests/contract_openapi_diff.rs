//! OpenAPI contract diff-test.
//!
//! Ensures the `utoipa` export from apxm-server matches the checked-in contract
//! baseline at `contracts/session-api.yaml`.

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use apxm_server::openapi::session_api_openapi_yaml;
use serde_json::{Map, Value};

fn contract_baseline_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../contracts/session-api.yaml")
}

fn server_api_contract_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../contracts/server-api.yaml")
}

fn parse_yaml(value: &str) -> Value {
    serde_yaml::from_str(value).expect("valid OpenAPI YAML")
}

fn schema_names(doc: &Value) -> BTreeSet<String> {
    doc.pointer("/components/schemas")
        .and_then(Value::as_object)
        .map(|schemas| schemas.keys().cloned().collect())
        .unwrap_or_default()
}

fn resolve_ref(doc: &Value, reference: &str) -> Option<Value> {
    let name = reference.strip_prefix("#/components/schemas/")?;
    doc.pointer(&format!("/components/schemas/{name}")).cloned()
}

fn normalize_nullable(obj: &mut Map<String, Value>) {
    if let Some(Value::Array(types)) = obj.get("type")
        && types.len() == 2
        && types.iter().any(|t| t.as_str() == Some("null"))
        && let Some(non_null) = types.iter().find(|t| t.as_str() != Some("null"))
    {
        obj.insert("type".to_string(), non_null.clone());
        obj.insert("nullable".to_string(), Value::Bool(true));
    }
}

fn normalize_schema(doc: &Value, schema: &mut Value) {
    if let Some(reference) = schema
        .get("$ref")
        .and_then(Value::as_str)
        .and_then(|r| resolve_ref(doc, r))
    {
        *schema = reference;
    }

    let Some(obj) = schema.as_object_mut() else {
        return;
    };

    obj.remove("description");
    obj.remove("minimum");
    obj.remove("propertyNames");

    normalize_nullable(obj);

    if let Some(props) = obj.get_mut("properties").and_then(Value::as_object_mut) {
        for prop in props.values_mut() {
            normalize_schema(doc, prop);
        }
    }
    if let Some(items) = obj.get_mut("items") {
        normalize_schema(doc, items);
    }
    if let Some(additional) = obj.get_mut("additionalProperties")
        && additional.is_object()
    {
        normalize_schema(doc, additional);
    }
}

fn normalize_operation(doc: &Value, op: &mut Map<String, Value>) {
    op.remove("summary");
    op.remove("description");
    op.remove("tags");
    op.remove("parameters");

    if let Some(body) = op.get_mut("requestBody").and_then(Value::as_object_mut) {
        body.remove("description");
    }

    if let Some(responses) = op.get_mut("responses").and_then(Value::as_object_mut) {
        for response in responses.values_mut() {
            if let Some(response_obj) = response.as_object_mut() {
                response_obj.remove("description");
            }
            if let Some(content) = response.get_mut("content").and_then(Value::as_object_mut) {
                for media in content.values_mut() {
                    if let Some(schema) = media.get_mut("schema") {
                        normalize_schema(doc, schema);
                    }
                }
            }
        }
    }
}

/// Normalize OpenAPI documents for structural comparison (wire-relevant fields only).
fn normalize_openapi(mut doc: Value) -> Value {
    let lookup = doc.clone();
    let obj = doc.as_object_mut().expect("openapi root object");
    obj.remove("info");
    obj.remove("tags");
    obj.insert("openapi".to_string(), Value::String("3.0.3".to_string()));

    if let Some(components) = obj.get_mut("components").and_then(Value::as_object_mut)
        && let Some(schemas) = components.get_mut("schemas").and_then(Value::as_object_mut)
    {
        let names: Vec<String> = schemas.keys().cloned().collect();
        for name in names {
            if let Some(schema) = schemas.get_mut(&name) {
                normalize_schema(&lookup, schema);
            }
        }
    }

    if let Some(paths) = obj.get_mut("paths").and_then(Value::as_object_mut) {
        for path_item in paths.values_mut() {
            if let Some(ops) = path_item.as_object_mut() {
                for op in ops.values_mut() {
                    if let Some(op_obj) = op.as_object_mut() {
                        normalize_operation(&lookup, op_obj);
                    }
                }
            }
        }
    }

    doc
}

fn comparable_schemas(doc: &Value, keep: &BTreeSet<String>) -> BTreeMap<String, Value> {
    doc.pointer("/components/schemas")
        .and_then(Value::as_object)
        .map(|schemas| {
            schemas
                .iter()
                .filter(|(name, _)| keep.contains(*name))
                .map(|(name, schema)| (name.clone(), schema.clone()))
                .collect()
        })
        .unwrap_or_default()
}

fn comparable_paths(doc: &Value) -> Value {
    doc.get("paths").cloned().unwrap_or(Value::Null)
}

#[test]
fn exported_openapi_matches_contract_baseline() {
    let baseline =
        std::fs::read_to_string(contract_baseline_path()).expect("contract baseline YAML exists");
    let exported = session_api_openapi_yaml();

    let baseline_doc = normalize_openapi(parse_yaml(&baseline));
    let exported_doc = normalize_openapi(parse_yaml(&exported));
    let baseline_schemas = schema_names(&baseline_doc);

    assert_eq!(
        comparable_paths(&exported_doc),
        comparable_paths(&baseline_doc),
        "OpenAPI path surface drifted from contract baseline"
    );
    assert_eq!(
        comparable_schemas(&exported_doc, &baseline_schemas),
        comparable_schemas(&baseline_doc, &baseline_schemas),
        "OpenAPI schema drifted from contract baseline.\n\
         Regenerate contracts/session-api.yaml after intentional wire changes.\n\
         --- exported ---\n{exported}\n\
         --- baseline ---\n{baseline}"
    );
}

#[test]
fn exported_openapi_includes_session_and_permission_paths() {
    let exported = parse_yaml(&session_api_openapi_yaml());
    let paths = exported
        .get("paths")
        .and_then(Value::as_object)
        .expect("paths object");

    for path in [
        "/v1/sessions/{session_id}/status",
        "/v1/sessions/{session_id}/cancel",
        "/v1/sessions/{session_id}/compact",
        "/v1/sessions/{session_id}/events",
        "/v1/sessions/{session_id}/events/stream",
        "/v1/permissions/{permission_id}/respond",
    ] {
        assert!(paths.contains_key(path), "missing path {path}");
    }
}

fn operation<'a>(doc: &'a Value, path: &str, method: &str) -> &'a Value {
    doc.get("paths")
        .and_then(Value::as_object)
        .and_then(|paths| paths.get(path))
        .and_then(|path_item| path_item.get(method))
        .unwrap_or_else(|| panic!("missing operation {method} {path}"))
}

fn parameter_names(operation: &Value) -> BTreeSet<String> {
    operation
        .get("parameters")
        .and_then(Value::as_array)
        .map(|params| {
            params
                .iter()
                .filter_map(|param| param.get("name").and_then(Value::as_str))
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

#[test]
fn checked_server_api_documents_run_observability_controls() {
    let baseline =
        std::fs::read_to_string(server_api_contract_path()).expect("server API YAML exists");
    let doc = parse_yaml(&baseline);

    let list_runs = operation(&doc, "/v1/runs", "get");
    let run_params = parameter_names(list_runs);
    for name in ["session_id", "trace_id", "status", "limit"] {
        assert!(
            run_params.contains(name),
            "GET /v1/runs missing query parameter {name}"
        );
    }

    let clear_runs = operation(&doc, "/v1/runs/clear", "post");
    assert_eq!(
        clear_runs.get("operationId").and_then(Value::as_str),
        Some("clearRuns"),
        "POST /v1/runs/clear must be documented"
    );
    assert!(
        clear_runs
            .pointer("/responses/200/content/application~1json/schema/properties/cleared")
            .is_some(),
        "clear runs response must document cleared count"
    );

    let events = operation(&doc, "/v1/runs/{execution_id}/events", "get");
    let event_params = parameter_names(events);
    for name in ["since", "limit"] {
        assert!(
            event_params.contains(name),
            "GET /v1/runs/{{id}}/events missing query parameter {name}"
        );
    }

    let stream = operation(&doc, "/v1/runs/{execution_id}/events/stream", "get");
    let stream_params = parameter_names(stream);
    for name in ["since", "Last-Event-ID"] {
        assert!(
            stream_params.contains(name),
            "GET /v1/runs/{{id}}/events/stream missing replay parameter {name}"
        );
    }
    assert!(
        !stream_params.contains("limit"),
        "SSE stream must not document limit until the handler enforces it"
    );

    let artifact = operation(
        &doc,
        "/v1/runs/{execution_id}/artifacts/{artifact_path}",
        "get",
    );
    assert!(
        artifact.pointer("/responses/400").is_some(),
        "artifact fetch must document invalid-path 400"
    );
    let greedy = artifact
        .get("parameters")
        .and_then(Value::as_array)
        .and_then(|params| {
            params
                .iter()
                .find(|param| param.get("name").and_then(Value::as_str) == Some("artifact_path"))
        })
        .expect("artifact_path parameter");
    assert_eq!(
        greedy.pointer("/x-apxm-greedy-path"),
        Some(&Value::Bool(true)),
        "artifact_path must declare greedy path semantics"
    );
}
