use std::collections::hash_map::DefaultHasher;
use std::fs;
use std::hash::{Hash, Hasher};
use std::path::Path;

use anyhow::Result;

use apxm_core::types::{
    WORKFLOW_SPAWN_PATH_TARGET_KINDS, WORKFLOW_TARGET_KIND_AIR_PATH,
    WORKFLOW_TARGET_KIND_ARTIFACT_PATH, WORKFLOW_TARGET_KIND_REGISTERED_FLOW,
    WORKFLOW_TARGET_KIND_WORKFLOW_PATH,
};

use super::registry::{
    FrontendAgentTemplate, FrontendConstant, FrontendModelSpec, agent_templates, builtin_models,
    builtin_providers, graph_attr_constants, graph_metadata_constants, graph_metric_constants,
    operation_specs, provider_protocols, valid_param_types,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GeneratedFrontendFiles {
    pub constants_py: String,
    pub operations_py: String,
    pub agents_py: String,
    pub providers_py: String,
    pub models_py: String,
}

pub fn render_generated_files() -> GeneratedFrontendFiles {
    GeneratedFrontendFiles {
        constants_py: stamp_codegen_hash(&render_constants_module()),
        operations_py: stamp_codegen_hash(&render_operations_module()),
        agents_py: stamp_codegen_hash(&render_agents_module()),
        providers_py: stamp_codegen_hash(&render_providers_module()),
        models_py: stamp_codegen_hash(&render_models_module()),
    }
}

pub fn render_generated_init() -> String {
    let mut buf = String::new();
    buf.push_str("\"\"\"Auto-generated APXM frontend bindings.\"\"\"\n\n");
    buf.push_str("from .agents import *\n");
    buf.push_str("from .constants import *\n");
    buf.push_str("from .models import *\n");
    buf.push_str("from .operations import *\n");
    buf.push_str("from .providers import *\n");
    buf
}

pub fn render_generated_python() -> Vec<(&'static str, String)> {
    let rendered = render_generated_files();
    vec![
        ("__init__.py", render_generated_init()),
        ("constants.py", rendered.constants_py),
        ("operations.py", rendered.operations_py),
        ("agents.py", rendered.agents_py),
        ("models.py", rendered.models_py),
        ("providers.py", rendered.providers_py),
    ]
}

pub fn render_runtime_evidence_python() -> String {
    let runtime_kinds = apxm_program::FactKind::all()
        .iter()
        .map(|kind| py_string(kind.wire()))
        .collect::<Vec<_>>()
        .join(", ");
    let runtime_schema = py_string(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../../contracts/schemas/apxm.runtime-evidence.v1.json"
    )));
    let common_schema = py_string(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../../../contracts/schemas/contract-common.v1.json"
    )));
    format!(
        r##"# AUTO-GENERATED from apxm.runtime-evidence.v1; DO NOT EDIT.
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

pub fn write_generated_python(output_dir: impl AsRef<Path>) -> Result<()> {
    let output_dir = output_dir.as_ref();
    fs::create_dir_all(output_dir)?;

    for (filename, content) in render_generated_python() {
        fs::write(output_dir.join(filename), content)?;
    }

    Ok(())
}

fn render_constants_module() -> String {
    let mut buf = String::new();
    buf.push_str("# AUTO-GENERATED by compiler frontend; DO NOT EDIT.\n");
    buf.push_str("from typing import Final\n\n");

    buf.push_str("# Graph metadata keys\n");
    for item in graph_metadata_constants() {
        render_constant(&mut buf, &item);
    }

    buf.push_str("\n# Graph attribute keys\n");
    for item in graph_attr_constants() {
        render_constant(&mut buf, &item);
    }

    buf.push_str("\n# Graph metrics keys\n");
    for item in graph_metric_constants() {
        render_constant(&mut buf, &item);
    }

    buf.push_str("\n# Workflow target kinds\n");
    buf.push_str(&format!(
        "WORKFLOW_TARGET_KIND_REGISTERED_FLOW: Final[str] = {}\n",
        py_string(WORKFLOW_TARGET_KIND_REGISTERED_FLOW)
    ));
    buf.push_str(&format!(
        "WORKFLOW_TARGET_KIND_AIR_PATH: Final[str] = {}\n",
        py_string(WORKFLOW_TARGET_KIND_AIR_PATH)
    ));
    buf.push_str(&format!(
        "WORKFLOW_TARGET_KIND_ARTIFACT_PATH: Final[str] = {}\n",
        py_string(WORKFLOW_TARGET_KIND_ARTIFACT_PATH)
    ));
    buf.push_str(&format!(
        "WORKFLOW_TARGET_KIND_WORKFLOW_PATH: Final[str] = {}\n",
        py_string(WORKFLOW_TARGET_KIND_WORKFLOW_PATH)
    ));
    let workflow_spawn_target_kinds = WORKFLOW_SPAWN_PATH_TARGET_KINDS
        .iter()
        .map(|kind| py_string(kind))
        .collect::<Vec<_>>()
        .join(", ");
    buf.push_str(&format!(
        "WORKFLOW_SPAWN_PATH_TARGET_KINDS: Final[tuple[str, ...]] = ({workflow_spawn_target_kinds})\n"
    ));

    // Valid parameter types
    let types = valid_param_types();
    let types_set = types
        .iter()
        .map(|t| py_string(t))
        .collect::<Vec<_>>()
        .join(", ");
    buf.push_str(&format!(
        "\nVALID_PARAM_TYPES: Final[frozenset[str]] = frozenset({{{types_set}}})\n"
    ));

    // Python type → APXM type mapping (derived from valid param types + Python aliases)
    let mut mapping_parts = Vec::new();
    for t in types {
        mapping_parts.push(format!("{}: {}", py_string(t), py_string(t)));
    }
    // Additional Python-specific aliases that map to APXM "json"
    mapping_parts.push(format!("{}: {}", py_string("dict"), py_string("json")));
    mapping_parts.push(format!("{}: {}", py_string("list"), py_string("json")));
    mapping_parts.push(format!("{}: {}", py_string("Any"), py_string("json")));
    buf.push_str(&format!(
        "PYTHON_TYPE_TO_APXM: Final[dict[str, str]] = {{{}}}\n",
        mapping_parts.join(", ")
    ));

    buf
}

fn render_operations_module() -> String {
    let mut buf = String::new();
    buf.push_str("# AUTO-GENERATED by compiler frontend; DO NOT EDIT.\n");
    buf.push_str("from dataclasses import dataclass\n");
    buf.push_str("from typing import Final\n\n");

    buf.push_str("@dataclass(frozen=True)\n");
    buf.push_str("class FieldSpec:\n");
    buf.push_str("    name: str\n");
    buf.push_str("    description: str\n");
    buf.push_str("    required: bool\n");
    buf.push_str("    ref_type: str | None\n\n");

    buf.push_str("@dataclass(frozen=True)\n");
    buf.push_str("class OpSpec:\n");
    buf.push_str("    op: str\n");
    buf.push_str("    name: str\n");
    buf.push_str("    category: str\n");
    buf.push_str("    description: str\n");
    buf.push_str("    long_description: str\n");
    buf.push_str("    latency: str\n");
    buf.push_str("    fields: tuple[FieldSpec, ...]\n");
    buf.push_str("    produces_output: bool\n");
    buf.push_str("    needs_submission: bool\n");
    buf.push_str("    min_inputs: int\n");
    buf.push_str("    example_json: str | None\n\n");

    let ops = operation_specs();
    for spec in &ops {
        let op = spec.op.to_string();
        let ident = py_identifier(&op).to_ascii_uppercase();
        buf.push_str(&format!("{ident}: Final = OpSpec(\n"));
        buf.push_str(&format!("    op={},\n", py_string(&op)));
        buf.push_str(&format!("    name={},\n", py_string(spec.name)));
        buf.push_str(&format!("    category={},\n", py_string(spec.category)));
        buf.push_str(&format!(
            "    description={},\n",
            py_string(spec.description)
        ));
        buf.push_str(&format!(
            "    long_description={},\n",
            py_string(spec.long_description)
        ));
        buf.push_str(&format!("    latency={},\n", py_string(spec.latency)));
        buf.push_str(&format!(
            "    fields={},\n",
            py_field_specs_tuple(&spec.fields)
        ));
        buf.push_str(&format!(
            "    produces_output={},\n",
            py_bool(spec.produces_output)
        ));
        buf.push_str(&format!(
            "    needs_submission={},\n",
            py_bool(spec.needs_submission)
        ));
        buf.push_str(&format!("    min_inputs={},\n", spec.min_inputs));
        buf.push_str(&format!(
            "    example_json={},\n",
            py_optional_string(spec.example_json)
        ));
        buf.push_str(")\n\n");
    }

    buf.push_str("ALL_OPERATIONS: Final[tuple[OpSpec, ...]] = (\n");
    for spec in &ops {
        let op = spec.op.to_string();
        let ident = py_identifier(&op).to_ascii_uppercase();
        buf.push_str(&format!("    {ident},\n"));
    }
    buf.push_str(")\n\n");

    // Category constants
    let categories = [
        ("CATEGORY_METADATA", "metadata"),
        ("CATEGORY_MEMORY", "memory"),
        ("CATEGORY_REASONING", "reasoning"),
        ("CATEGORY_TOOLS", "tools"),
        ("CATEGORY_CONTROL_FLOW", "control_flow"),
        ("CATEGORY_SYNCHRONIZATION", "synchronization"),
        ("CATEGORY_ERROR_HANDLING", "error_handling"),
        ("CATEGORY_COMMUNICATION", "communication"),
        ("CATEGORY_COORDINATION", "coordination"),
        ("CATEGORY_IDENTITY", "identity"),
        ("CATEGORY_INTERNAL", "internal"),
    ];
    for (name, value) in &categories {
        buf.push_str(&format!("{name}: Final[str] = {}\n", py_string(value)));
    }

    // OP_* constants
    buf.push('\n');
    for spec in &ops {
        let op = spec.op.to_string();
        let const_name = format!("OP_{}", py_identifier(&op).to_ascii_uppercase());
        buf.push_str(&format!("{const_name}: Final[str] = {}\n", py_string(&op)));
    }

    // LLM_OPS — pre-computed from reasoning category
    let llm_ops: Vec<String> = ops
        .iter()
        .filter(|s| s.category == "reasoning")
        .map(|s| s.op.to_string())
        .collect::<Vec<_>>();
    buf.push_str(&format!(
        "\nLLM_OPS: Final[frozenset] = frozenset({{{}}})\n",
        llm_ops
            .iter()
            .map(|op| py_string(op))
            .collect::<Vec<_>>()
            .join(", "),
    ));

    buf
}

fn render_agents_module() -> String {
    let mut buf = String::new();
    buf.push_str("# AUTO-GENERATED by compiler frontend; DO NOT EDIT.\n");
    buf.push_str("from dataclasses import dataclass\n");
    buf.push_str("from typing import Final\n\n");
    buf.push_str("@dataclass(frozen=True)\n");
    buf.push_str("class AgentRef:\n");
    buf.push_str("    name: str\n");
    buf.push_str("    command: str\n");
    buf.push_str("    description: str | None\n");
    buf.push_str("    route_capabilities: tuple[str, ...]\n");
    buf.push_str("    source: str\n");
    buf.push_str("    default_mode: str | None\n");
    buf.push_str("    default_model: str | None\n\n");

    let templates = agent_templates();
    for item in &templates {
        let ident = py_identifier(&item.name);
        render_agent_ref(&mut buf, &ident, item);
    }

    buf.push_str("ALL_AGENTS: Final[tuple[AgentRef, ...]] = (\n");
    for item in &templates {
        buf.push_str(&format!("    {},\n", py_identifier(&item.name)));
    }
    buf.push_str(")\n");
    buf
}

fn render_providers_module() -> String {
    let mut buf = String::new();
    buf.push_str("# AUTO-GENERATED by compiler frontend; DO NOT EDIT.\n");
    buf.push_str("from dataclasses import dataclass\n");
    buf.push_str("from typing import Final\n\n");

    buf.push_str("@dataclass(frozen=True)\n");
    buf.push_str("class ProviderSpec:\n");
    buf.push_str("    id: str\n");
    buf.push_str("    protocol: str\n");
    buf.push_str("    default_base_url: str | None\n");
    buf.push_str("    requires_api_key: bool\n");
    buf.push_str("    api_key_env_var: str | None\n");
    buf.push_str("    aliases: tuple[str, ...]\n\n");

    let providers = builtin_providers();
    for p in &providers {
        let ident = p.id.to_ascii_uppercase();
        buf.push_str(&format!("{ident}: Final = ProviderSpec(\n"));
        buf.push_str(&format!("    id={},\n", py_string(p.id)));
        buf.push_str(&format!("    protocol={},\n", py_string(p.protocol)));
        buf.push_str(&format!(
            "    default_base_url={},\n",
            py_optional_string(p.default_base_url)
        ));
        buf.push_str(&format!(
            "    requires_api_key={},\n",
            py_bool(p.requires_api_key)
        ));
        buf.push_str(&format!(
            "    api_key_env_var={},\n",
            py_optional_string(p.api_key_env_var)
        ));
        buf.push_str(&format!(
            "    aliases={},\n",
            py_static_str_tuple(p.aliases)
        ));
        buf.push_str(")\n\n");
    }

    buf.push_str("BUILTIN_PROVIDERS: Final[tuple[ProviderSpec, ...]] = (\n");
    for p in &providers {
        let ident = p.id.to_ascii_uppercase();
        buf.push_str(&format!("    {ident},\n"));
    }
    buf.push_str(")\n\n");

    let protocols = provider_protocols();
    buf.push_str(&format!(
        "PROVIDER_PROTOCOLS: Final[tuple[str, ...]] = {}\n",
        py_static_str_tuple(&protocols)
    ));

    // REGISTERED_PROVIDERS lookup dict (canonical name + aliases)
    buf.push_str("\nREGISTERED_PROVIDERS: dict[str, ProviderSpec] = {}\n");
    buf.push_str("for _p in BUILTIN_PROVIDERS:\n");
    buf.push_str("    REGISTERED_PROVIDERS[_p.id] = _p\n");
    buf.push_str("    for _a in _p.aliases:\n");
    buf.push_str("        REGISTERED_PROVIDERS[_a] = _p\n");
    buf.push('\n');

    // resolve_provider()
    buf.push_str("def resolve_provider(name: str) -> ProviderSpec:\n");
    buf.push_str("    spec = REGISTERED_PROVIDERS.get(name.lower())\n");
    buf.push_str("    if spec is None:\n");
    buf.push_str("        valid = sorted({p.id for p in BUILTIN_PROVIDERS})\n");
    buf.push_str("        raise ValueError(\n");
    buf.push_str("            f\"Unknown provider '{name}'. \"\n");
    buf.push_str("            f\"Registered APXM providers: {', '.join(valid)}. \"\n");
    buf.push_str("            f\"To add a custom provider, register it in apxm-core and apxm-backends first.\"\n");
    buf.push_str("        )\n");
    buf.push_str("    return spec\n\n");

    // list_providers()
    buf.push_str("def list_providers() -> list[str]:\n");
    buf.push_str("    return sorted({p.id for p in BUILTIN_PROVIDERS})\n");

    buf
}

fn render_models_module() -> String {
    let mut buf = String::new();
    buf.push_str("# AUTO-GENERATED by compiler frontend; DO NOT EDIT.\n");
    buf.push_str("from typing import Final\n\n\n");

    // ModelId class
    buf.push_str("class ModelId:\n");
    buf.push_str("    \"\"\"Typed model identifier.\"\"\"\n");
    buf.push_str("    __slots__ = (\"_value\",)\n\n");
    buf.push_str("    def __init__(self, value: str) -> None:\n");
    buf.push_str("        self._value = value\n\n");
    buf.push_str("    def __str__(self) -> str:\n");
    buf.push_str("        return self._value\n\n");
    buf.push_str("    def __repr__(self) -> str:\n");
    buf.push_str("        return f\"ModelId({self._value!r})\"\n\n");
    buf.push_str("    def __eq__(self, other: object) -> bool:\n");
    buf.push_str("        if isinstance(other, ModelId):\n");
    buf.push_str("            return self._value == other._value\n");
    buf.push_str("        if isinstance(other, str):\n");
    buf.push_str("            return self._value == other\n");
    buf.push_str("        return NotImplemented\n\n");
    buf.push_str("    def __hash__(self) -> int:\n");
    buf.push_str("        return hash(self._value)\n\n\n");

    // Group models by provider
    let models = builtin_models();
    let mut providers: Vec<&str> = models.iter().map(|m| m.provider).collect();
    providers.sort_unstable();
    providers.dedup();

    for provider in &providers {
        let class_name = provider_class_name(provider);
        buf.push_str(&format!("class {class_name}:\n"));

        let provider_models: Vec<&FrontendModelSpec> =
            models.iter().filter(|m| m.provider == *provider).collect();

        for m in &provider_models {
            let const_name = to_model_constant(m.id);
            buf.push_str(&format!(
                "    {const_name}: Final[ModelId] = ModelId({id})\n",
                id = py_string(m.id)
            ));
        }

        buf.push_str("\n\n");
    }

    // ALL_MODELS tuple
    buf.push_str("ALL_MODELS: Final[tuple[ModelId, ...]] = (\n");
    for provider in &providers {
        let class_name = provider_class_name(provider);
        let provider_models: Vec<&FrontendModelSpec> =
            models.iter().filter(|m| m.provider == *provider).collect();
        for m in &provider_models {
            let const_name = to_model_constant(m.id);
            buf.push_str(&format!("    {class_name}.{const_name},\n"));
        }
    }
    buf.push_str(")\n");

    buf
}

/// Convert a model id to a Python constant name.
///
/// Rules:
/// - Replace `-` and `.` with `_`
/// - Strip date suffixes (e.g. `-20241022`, `-20240229`)
/// - Uppercase
fn to_model_constant(id: &str) -> String {
    // Hugging-Face style ids (`vendor/name`) get a richer transform so that
    // e.g. "Qwen/Qwen2.5-7B-Instruct" becomes "QWEN_2_5_7B".
    let has_vendor_prefix = id.contains('/');
    let after_slash = id.rsplit('/').next().unwrap_or(id);

    // Strip common suffix tokens (instruction-tuned variants).
    let trimmed_suffix = ["-Instruct", "-instruct", "-Chat", "-chat"]
        .iter()
        .fold(after_slash, |acc, suf| acc.strip_suffix(suf).unwrap_or(acc));

    // Strip trailing date suffixes like -20241022
    let stripped = if trimmed_suffix.len() > 9 {
        let suffix = &trimmed_suffix[trimmed_suffix.len() - 9..];
        if suffix.starts_with('-')
            && suffix[1..].chars().all(|c| c.is_ascii_digit())
            && suffix.len() == 9
        {
            &trimmed_suffix[..trimmed_suffix.len() - 9]
        } else {
            trimmed_suffix
        }
    } else {
        trimmed_suffix
    };

    // For HF-style ids, insert separator between a letter and a following
    // digit so "Qwen2.5" → "QWEN_2_5" and "Llama3.1" → "LLAMA_3_1".
    // Skipped for non-vendor ids to preserve names like "GPT_4O" and "O1_MINI".
    let spaced = if has_vendor_prefix {
        let mut buf = String::with_capacity(stripped.len() + 4);
        let mut prev: Option<char> = None;
        for ch in stripped.chars() {
            if let Some(p) = prev
                && p.is_ascii_alphabetic()
                && ch.is_ascii_digit()
            {
                buf.push('-');
            }
            buf.push(ch);
            prev = Some(ch);
        }
        buf
    } else {
        stripped.to_string()
    };

    spaced.replace(['-', '.', '/'], "_").to_ascii_uppercase()
}

/// Map provider id to a Python class name.
fn provider_class_name(provider: &str) -> String {
    match provider {
        "openai" => "OpenAI".to_string(),
        _ => {
            let mut chars = provider.chars();
            match chars.next() {
                Some(first) => {
                    let mut name = first.to_uppercase().to_string();
                    name.extend(chars);
                    name
                }
                None => provider.to_string(),
            }
        }
    }
}

fn render_constant(buf: &mut String, item: &FrontendConstant) {
    buf.push_str(&format!(
        "{}: Final[str] = {}\n",
        item.name,
        py_string(item.value)
    ));
}

fn render_agent_ref(buf: &mut String, ident: &str, item: &FrontendAgentTemplate) {
    buf.push_str(&format!("{ident}: Final = AgentRef(\n"));
    buf.push_str(&format!("    name={},\n", py_string(&item.name)));
    buf.push_str(&format!("    command={},\n", py_string(&item.command)));
    buf.push_str(&format!(
        "    description={},\n",
        py_optional_string(item.description.as_deref())
    ));
    buf.push_str("    route_capabilities=(\n");
    for capability in &item.route_capabilities {
        buf.push_str(&format!("        {},\n", py_string(capability)));
    }
    buf.push_str("    ),\n");
    buf.push_str(&format!("    source={},\n", py_string(&item.source)));
    buf.push_str(&format!(
        "    default_mode={},\n",
        py_optional_string(item.default_mode.as_deref())
    ));
    buf.push_str(&format!(
        "    default_model={},\n",
        py_optional_string(item.default_model.as_deref())
    ));
    buf.push_str(")\n\n");
}

fn py_identifier(value: &str) -> String {
    let mut result = String::new();

    for (idx, ch) in value.chars().enumerate() {
        let mapped = if ch.is_ascii_alphanumeric() || ch == '_' {
            ch
        } else {
            '_'
        };

        if idx == 0 && mapped.is_ascii_digit() {
            result.push('_');
        }

        result.push(mapped.to_ascii_lowercase());
    }

    if result.is_empty() {
        "_generated".to_string()
    } else {
        result
    }
}

fn py_optional_string(value: Option<&str>) -> String {
    match value {
        Some(value) => py_string(value),
        None => "None".to_string(),
    }
}

fn py_string(value: &str) -> String {
    serde_json::to_string(value).expect("python string literal")
}

fn py_static_str_tuple(values: &[&str]) -> String {
    match values.len() {
        0 => "()".to_string(),
        1 => format!("({},)", py_string(values[0])),
        _ => {
            let joined = values
                .iter()
                .map(|value| py_string(value))
                .collect::<Vec<_>>()
                .join(", ");
            format!("({joined})")
        }
    }
}

fn py_field_specs_tuple(fields: &[super::registry::FrontendFieldSpec]) -> String {
    if fields.is_empty() {
        return "()".to_string();
    }
    let mut parts = Vec::new();
    for f in fields {
        parts.push(format!(
            "FieldSpec(name={}, description={}, required={}, ref_type={})",
            py_string(f.name),
            py_string(f.description),
            py_bool(f.required),
            py_optional_string(f.ref_type),
        ));
    }
    if parts.len() == 1 {
        format!("({},)", parts[0])
    } else {
        format!("({})", parts.join(", "))
    }
}

fn py_bool(value: bool) -> &'static str {
    if value { "True" } else { "False" }
}

/// Compute a content hash and prepend it as a `_CODEGEN_HASH` constant.
///
/// The hash covers the body content (everything after the header), so any
/// manual edit to a generated file can be detected at import time.
fn stamp_codegen_hash(body: &str) -> String {
    let mut hasher = DefaultHasher::new();
    body.hash(&mut hasher);
    let hash = hasher.finish();
    format!(
        "# AUTO-GENERATED by compiler frontend; DO NOT EDIT.\n_CODEGEN_HASH: str = \"{:016x}\"\n{}",
        hash,
        // Strip a preexisting header line so we don't duplicate
        body.strip_prefix("# AUTO-GENERATED by compiler frontend; DO NOT EDIT.\n")
            .unwrap_or(body),
    )
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
