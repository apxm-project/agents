//! Python frontend codegen: the generated runtime-evidence fact binding.

pub fn render_runtime_evidence_python() -> String {
    let runtime_kinds = apxm_program::FactKind::all()
        .iter()
        .map(|kind| py_string(kind.wire()))
        .collect::<Vec<_>>()
        .join(", ");
    let runtime_schema = py_string(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../../contracts/schemas/apxm.runtime-evidence.json"
    )));
    let common_schema = py_string(include_str!(env!("APXM_CONTRACT_COMMON_SCHEMA_PATH")));
    format!(
        r##"# AUTO-GENERATED from apxm.runtime-evidence; DO NOT EDIT.
import json
import re
from dataclasses import dataclass
from typing import Any, Final, TypeAlias

RUNTIME_FACT_KINDS: Final[frozenset[str]] = frozenset(({runtime_kinds}))
LOOP_ITERATION_COMPLETED: Final[str] = "LoopIterationCompleted"
NODE_EXECUTION_RECORDED: Final[str] = "node_execution.recorded"
ALL_FACT_KINDS: Final[frozenset[str]] = RUNTIME_FACT_KINDS | frozenset((LOOP_ITERATION_COMPLETED,))
_SCHEMA: Final[dict[str, Any]] = json.loads({runtime_schema})
_COMMON: Final[dict[str, Any]] = json.loads({common_schema})

@dataclass(frozen=True)
class RuntimeFact:
    fact_id: str
    event_sequence: int
    fact_kind: str
    optional_fields: dict[str, Any]

    def to_dict(self) -> dict[str, Any]:
        return {{"fact_id": self.fact_id, "event_sequence": self.event_sequence, "fact_kind": self.fact_kind, **self.optional_fields}}

@dataclass(frozen=True)
class NodeExecutionRecordedFact:
    fact_id: str
    event_sequence: int
    node_execution_id: str
    air_node_id: str
    execution_scope: dict[str, Any]
    parent_node_execution_id: str | None = None

@dataclass(frozen=True)
class LoopIterationCompletedFact:
    fact_id: str
    event_sequence: int
    static_loop_id: str
    loop_occurrence_id: str
    iteration_index: int
    program_invocation_id: str
    causal_node_execution_ids: tuple[str, ...]

Fact: TypeAlias = RuntimeFact | NodeExecutionRecordedFact | LoopIterationCompletedFact

def _resolve(ref: str, root: dict[str, Any]) -> tuple[dict[str, Any], dict[str, Any]]:
    if ref.startswith("#/$defs/"):
        return root["$defs"][ref.removeprefix("#/$defs/")], root
    marker = "apxm.contract-common.v1#/$defs/"
    if ref.startswith(marker):
        return _COMMON["$defs"][ref.removeprefix(marker)], _COMMON
    raise ValueError(f"unsupported schema ref: {{ref}}")

def _validate(schema: dict[str, Any], value: Any, root: dict[str, Any], path: str) -> list[str]:
    if "$ref" in schema:
        target, target_root = _resolve(schema["$ref"], root)
        return _validate(target, value, target_root, path)
    if "oneOf" in schema:
        results = [_validate(branch, value, root, path) for branch in schema["oneOf"]]
        matches = sum(not errors for errors in results)
        if matches != 1:
            return [f"{{path}}: expected exactly one oneOf branch, matched {{matches}}", *[error for errors in results for error in errors]]
    errors: list[str] = []
    if "const" in schema and value != schema["const"]:
        errors.append(f"{{path}}: const mismatch")
    if "enum" in schema and value not in schema["enum"]:
        errors.append(f"{{path}}: unknown enum value {{value!r}}")
    declared = schema.get("type")
    type_ok = {{
        "object": isinstance(value, dict),
        "array": isinstance(value, list),
        "string": isinstance(value, str),
        "integer": isinstance(value, int) and not isinstance(value, bool),
        "number": isinstance(value, (int, float)) and not isinstance(value, bool),
        "boolean": isinstance(value, bool),
        "null": value is None,
    }}
    if isinstance(declared, str) and not type_ok.get(declared, True):
        return [f"{{path}}: expected {{declared}}"]
    if isinstance(value, str):
        if "minLength" in schema and len(value) < schema["minLength"]: errors.append(f"{{path}}: shorter than minLength")
        if "maxLength" in schema and len(value) > schema["maxLength"]: errors.append(f"{{path}}: exceeds maxLength")
        if "pattern" in schema and re.fullmatch(schema["pattern"], value) is None: errors.append(f"{{path}}: pattern mismatch")
    if isinstance(value, (int, float)) and not isinstance(value, bool):
        if "minimum" in schema and value < schema["minimum"]: errors.append(f"{{path}}: below minimum")
        if "maximum" in schema and value > schema["maximum"]: errors.append(f"{{path}}: above maximum")
    if isinstance(value, list):
        if "minItems" in schema and len(value) < schema["minItems"]: errors.append(f"{{path}}: fewer than minItems")
        if "maxItems" in schema and len(value) > schema["maxItems"]: errors.append(f"{{path}}: exceeds maxItems")
        if schema.get("uniqueItems") and len({{json.dumps(item, sort_keys=True, separators=(',', ':')) for item in value}}) != len(value): errors.append(f"{{path}}: duplicate items")
        if "items" in schema:
            for index, item in enumerate(value): errors.extend(_validate(schema["items"], item, root, f"{{path}}[{{index}}]"))
    if isinstance(value, dict):
        required = schema.get("required", [])
        for key in required:
            if key not in value: errors.append(f"{{path}}.{{key}}: required")
        properties = schema.get("properties", {{}})
        for key, item in value.items():
            if key in properties: errors.extend(_validate(properties[key], item, root, f"{{path}}.{{key}}"))
            elif schema.get("additionalProperties") is False: errors.append(f"{{path}}: unknown field {{key}}")
    return errors

def decode_fact(value: dict[str, Any]) -> Fact:
    errors = _validate(_SCHEMA["$defs"]["Fact"], value, _SCHEMA, "$")
    if errors:
        raise ValueError("; ".join(errors))
    kind = value["fact_kind"]
    if kind == LOOP_ITERATION_COMPLETED:
        return LoopIterationCompletedFact(value["fact_id"], value["event_sequence"], value["static_loop_id"], value["loop_occurrence_id"], value["iteration_index"], value["program_invocation_id"], tuple(value["causal_node_execution_ids"]))
    if kind == NODE_EXECUTION_RECORDED:
        return NodeExecutionRecordedFact(value["fact_id"], value["event_sequence"], value["node_execution_id"], value["air_node_id"], value["execution_scope"], value.get("parent_node_execution_id"))
    optional_fields = {{key: item for key, item in value.items() if key not in {{"fact_id", "event_sequence", "fact_kind"}}}}
    return RuntimeFact(value["fact_id"], value["event_sequence"], kind, optional_fields)
"##
    )
}

fn py_string(value: &str) -> String {
    serde_json::to_string(value).expect("python string literal")
}

#[cfg(test)]
mod evidence_tests {
    use super::*;

    #[test]
    fn python_codegen_emits_closed_runtime_evidence_decoder() {
        let evidence = render_runtime_evidence_python();
        for required in [
            "LoopIterationCompletedFact",
            "NodeExecutionRecordedFact",
            "decode_fact",
            "_SCHEMA",
            "unknown enum value",
        ] {
            assert!(evidence.contains(required), "{required}");
        }
    }
}
