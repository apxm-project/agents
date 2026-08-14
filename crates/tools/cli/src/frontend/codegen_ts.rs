use std::fs;
use std::path::Path;

use anyhow::Result;

use super::codegen_capabilities::TYPESCRIPT_CAPABILITIES_FILE;
use super::codegen_permissions::TYPESCRIPT_PERMISSIONS_FILE;
use super::registry::{
    FrontendOperationSpec, builtin_providers, graph_attr_constants, graph_metric_constants,
    operation_specs, provider_protocols,
};

pub const RUNTIME_EVIDENCE_TYPESCRIPT_FILE: &str = "runtime-evidence.ts";

/// Every file `@apxm/frontend`'s `src/generated` directory may contain. The
/// stray-file guard belongs to the directory rather than to one codegen arm:
/// `typescript-frontend` owns the runtime-evidence binding, `capabilities`
/// owns the capability catalogue, and `permissions` owns the permission
/// decision vocabulary — all three write here.
pub const GENERATED_TYPESCRIPT_FRONTEND_FILES: &[&str] = &[
    TYPESCRIPT_CAPABILITIES_FILE,
    TYPESCRIPT_PERMISSIONS_FILE,
    RUNTIME_EVIDENCE_TYPESCRIPT_FILE,
];

pub fn render_typescript_frontend_files() -> Vec<(&'static str, String)> {
    vec![(
        RUNTIME_EVIDENCE_TYPESCRIPT_FILE,
        render_runtime_evidence_typescript(),
    )]
}

fn render_runtime_evidence_typescript() -> String {
    let runtime_kinds = apxm_program::FactKind::all()
        .iter()
        .map(|kind| ts_string(kind.wire()))
        .collect::<Vec<_>>()
        .join(", ");
    let runtime_schema = ts_string(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../../contracts/schemas/apxm.runtime-evidence.json"
    )));
    let common_schema = ts_string(include_str!(env!("APXM_CONTRACT_COMMON_SCHEMA_PATH")));
    format!(
        r##"// AUTO-GENERATED from apxm.runtime-evidence; DO NOT EDIT.
export const RUNTIME_FACT_KINDS = [{runtime_kinds}] as const;
export const ALL_FACT_KINDS = [...RUNTIME_FACT_KINDS, "LoopIterationCompleted"] as const;
export type RuntimeFactKind = (typeof RUNTIME_FACT_KINDS)[number];
const runtimeSchema = JSON.parse({runtime_schema}) as Record<string, any>;
const commonSchema = JSON.parse({common_schema}) as Record<string, any>;

export type RuntimeFact = {{
  readonly fact_id: string;
  readonly event_sequence: number;
  readonly fact_kind: RuntimeFactKind;
}};
export type NodeExecutionScope =
  | {{ readonly scope_kind: "non_loop" }}
  | {{ readonly scope_kind: "loop"; readonly region_occurrence_id: string; readonly static_region_id: string; readonly loop_memberships: ReadonlyArray<{{ readonly static_loop_id: string; readonly loop_occurrence_id: string }}> }};
export type NodeExecutionRecordedFact = {{
  readonly fact_id: string;
  readonly event_sequence: number;
  readonly fact_kind: "node_execution.recorded";
  readonly node_execution_id: string;
  readonly air_node_id: string;
  readonly parent_node_execution_id?: string;
  readonly execution_scope: NodeExecutionScope;
}};
export type LoopIterationCompletedFact = {{
  readonly fact_id: string;
  readonly event_sequence: number;
  readonly fact_kind: "LoopIterationCompleted";
  readonly static_loop_id: string;
  readonly loop_occurrence_id: string;
  readonly iteration_index: number;
  readonly program_invocation_id: string;
  readonly causal_node_execution_ids: readonly [string, ...string[]];
}};
export type Fact = RuntimeFact | NodeExecutionRecordedFact | LoopIterationCompletedFact;

function resolveRef(ref: string, root: Record<string, any>): [Record<string, any>, Record<string, any>] {{
  if (ref.startsWith("#/$defs/")) return [root.$defs[ref.slice("#/$defs/".length)], root];
  const marker = "apxm.contract-common.v1#/$defs/";
  if (ref.startsWith(marker)) return [commonSchema.$defs[ref.slice(marker.length)], commonSchema];
  throw new Error(`unsupported schema ref: ${{ref}}`);
}}
function stable(value: unknown): string {{
  if (Array.isArray(value)) return `[${{value.map(stable).join(",")}}]`;
  if (value !== null && typeof value === "object") {{
    const record = value as Record<string, unknown>;
    return `{{${{Object.keys(record).sort().map((key) => `${{JSON.stringify(key)}}:${{stable(record[key])}}`).join(",")}}}}`;
  }}
  return JSON.stringify(value);
}}
function validate(schema: Record<string, any>, value: unknown, root: Record<string, any>, path: string): string[] {{
  if (schema.$ref) {{
    const [target, targetRoot] = resolveRef(schema.$ref, root);
    return validate(target, value, targetRoot, path);
  }}
  if (schema.oneOf) {{
    const results = schema.oneOf.map((branch: Record<string, any>) => validate(branch, value, root, path));
    const matches = results.filter((errors: string[]) => errors.length === 0).length;
    if (matches !== 1) return [`${{path}}: expected exactly one oneOf branch, matched ${{matches}}`, ...results.flat()];
  }}
  const errors: string[] = [];
  if ("const" in schema && value !== schema.const) errors.push(`${{path}}: const mismatch`);
  if (schema.enum && !schema.enum.includes(value)) errors.push(`${{path}}: unknown enum value ${{String(value)}}`);
  const typeMatches: Record<string, boolean> = {{
    object: value !== null && typeof value === "object" && !Array.isArray(value),
    array: Array.isArray(value), string: typeof value === "string",
    integer: typeof value === "number" && Number.isInteger(value),
    number: typeof value === "number", boolean: typeof value === "boolean", null: value === null,
  }};
  if (schema.type && !typeMatches[schema.type]) return [`${{path}}: expected ${{schema.type}}`];
  if (typeof value === "string") {{
    if (schema.minLength !== undefined && value.length < schema.minLength) errors.push(`${{path}}: shorter than minLength`);
    if (schema.maxLength !== undefined && value.length > schema.maxLength) errors.push(`${{path}}: exceeds maxLength`);
    if (schema.pattern && !(new RegExp(schema.pattern)).test(value)) errors.push(`${{path}}: pattern mismatch`);
  }}
  if (typeof value === "number") {{
    if (schema.minimum !== undefined && value < schema.minimum) errors.push(`${{path}}: below minimum`);
    if (schema.maximum !== undefined && value > schema.maximum) errors.push(`${{path}}: above maximum`);
  }}
  if (Array.isArray(value)) {{
    if (schema.minItems !== undefined && value.length < schema.minItems) errors.push(`${{path}}: fewer than minItems`);
    if (schema.maxItems !== undefined && value.length > schema.maxItems) errors.push(`${{path}}: exceeds maxItems`);
    if (schema.uniqueItems && new Set(value.map(stable)).size !== value.length) errors.push(`${{path}}: duplicate items`);
    if (schema.items) value.forEach((item, index) => errors.push(...validate(schema.items, item, root, `${{path}}[${{index}}]`)));
  }}
  if (value !== null && typeof value === "object" && !Array.isArray(value)) {{
    const record = value as Record<string, unknown>;
    for (const key of schema.required ?? []) if (!(key in record)) errors.push(`${{path}}.${{key}}: required`);
    for (const [key, item] of Object.entries(record)) {{
      if (schema.properties?.[key]) errors.push(...validate(schema.properties[key], item, root, `${{path}}.${{key}}`));
      else if (schema.additionalProperties === false) errors.push(`${{path}}: unknown field ${{key}}`);
    }}
  }}
  return errors;
}}
export function decodeFact(input: unknown): Fact {{
  const errors = validate(runtimeSchema.$defs.Fact, input, runtimeSchema, "$");
  if (errors.length > 0) throw new Error(errors.join("; "));
  return input as Fact;
}}
"##
    )
}

pub fn write_typescript_frontend_generated(output_dir: impl AsRef<Path>) -> Result<()> {
    let output_dir = output_dir.as_ref();
    fs::create_dir_all(output_dir)?;
    for (filename, content) in render_typescript_frontend_files() {
        fs::write(output_dir.join(filename), content)?;
    }
    Ok(())
}

pub fn render_generated_typescript() -> String {
    let mut buf = String::new();
    buf.push_str("// AUTO-GENERATED by apxm codegen typescript; DO NOT EDIT.\n\n");

    let ops = operation_specs();

    render_ts_field_spec(&mut buf);
    render_ts_op_spec(&mut buf);
    render_ts_dependency_types(&mut buf);
    render_ts_capability_groups(&mut buf);
    render_ts_operations(&mut buf, &ops);
    render_ts_categories(&mut buf, &ops);
    render_ts_attr_constants(&mut buf);
    render_ts_graph_metric_constants(&mut buf);
    render_ts_provider_types(&mut buf);

    buf
}

pub fn write_generated_typescript(output_path: impl AsRef<Path>) -> Result<()> {
    let output_path = output_path.as_ref();
    if let Some(parent) = output_path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(output_path, render_generated_typescript())?;
    Ok(())
}

fn render_ts_dependency_types(buf: &mut String) {
    buf.push_str("export enum DependencyType {\n");
    buf.push_str("  DATA = \"Data\",\n");
    buf.push_str("  CONTROL = \"Control\",\n");
    buf.push_str("  EFFECT = \"Effect\",\n");
    buf.push_str("}\n\n");
}

fn render_ts_capability_groups(buf: &mut String) {
    use apxm_core::constants::capabilities::groups;

    const CAPABILITY_GROUPS: &[(&str, &str)] = &[
        ("FILE", groups::FILE),
        ("FILE_READ", groups::FILE_READ),
        ("FILE_WRITE", groups::FILE_WRITE),
        ("HTTP", groups::HTTP),
        ("WEB", groups::WEB),
        ("SEARCH", groups::SEARCH),
        ("WEB_SEARCH", groups::WEB_SEARCH),
        ("SKILLS", groups::SKILLS),
        ("AUTHORING", groups::AUTHORING),
        ("TASK", groups::TASK),
        ("AGENT_MANAGEMENT", groups::AGENT_MANAGEMENT),
    ];

    buf.push_str("export enum ToolGroup {\n");
    for (ident, value) in CAPABILITY_GROUPS {
        buf.push_str(&format!("  {ident} = {},\n", ts_string(value)));
    }
    buf.push_str("}\n\n");
}

fn render_ts_field_spec(buf: &mut String) {
    buf.push_str("export type FieldSpec = {\n");
    buf.push_str("  readonly name: string;\n");
    buf.push_str("  readonly description: string;\n");
    buf.push_str("  readonly required: boolean;\n");
    buf.push_str("  readonly refType: string | null;\n");
    buf.push_str("};\n\n");
}

fn render_ts_op_spec(buf: &mut String) {
    buf.push_str("export type OpSpec = {\n");
    buf.push_str("  readonly op: string;\n");
    buf.push_str("  readonly name: string;\n");
    buf.push_str("  readonly category: OpCategory;\n");
    buf.push_str("  readonly description: string;\n");
    buf.push_str("  readonly longDescription: string;\n");
    buf.push_str("  readonly latency: string;\n");
    buf.push_str("  readonly fields: readonly FieldSpec[];\n");
    buf.push_str("  readonly producesOutput: boolean;\n");
    buf.push_str("  readonly needsSubmission: boolean;\n");
    buf.push_str("  readonly minInputs: number;\n");
    buf.push_str("  readonly exampleJson: string | null;\n");
    buf.push_str("};\n\n");
}

fn render_ts_operations(buf: &mut String, ops: &[FrontendOperationSpec]) {
    for spec in ops {
        let op = spec.op.to_string();
        let ident = op.to_ascii_uppercase();
        buf.push_str(&format!("export const {ident}: OpSpec = {{\n"));
        buf.push_str(&format!("  op: {},\n", ts_string(&op)));
        buf.push_str(&format!("  name: {},\n", ts_string(spec.name)));
        buf.push_str(&format!(
            "  category: {} as OpCategory,\n",
            ts_string(spec.category)
        ));
        buf.push_str(&format!(
            "  description: {},\n",
            ts_string(spec.description)
        ));
        buf.push_str(&format!(
            "  longDescription: {},\n",
            ts_string(spec.long_description)
        ));
        buf.push_str(&format!("  latency: {},\n", ts_string(spec.latency)));
        buf.push_str("  fields: [\n");
        for f in &spec.fields {
            buf.push_str(&format!(
                "    {{ name: {}, description: {}, required: {}, refType: {} }},\n",
                ts_string(f.name),
                ts_string(f.description),
                ts_bool(f.required),
                ts_optional_string(f.ref_type),
            ));
        }
        buf.push_str("  ],\n");
        buf.push_str(&format!(
            "  producesOutput: {},\n",
            ts_bool(spec.produces_output)
        ));
        buf.push_str(&format!(
            "  needsSubmission: {},\n",
            ts_bool(spec.needs_submission)
        ));
        buf.push_str(&format!("  minInputs: {},\n", spec.min_inputs));
        buf.push_str(&format!(
            "  exampleJson: {},\n",
            ts_optional_string(spec.example_json)
        ));
        buf.push_str("} as const;\n\n");
    }

    buf.push_str("export const ALL_OPERATIONS: readonly OpSpec[] = [\n");
    for spec in ops {
        buf.push_str(&format!(
            "  {},\n",
            spec.op.to_string().to_ascii_uppercase()
        ));
    }
    buf.push_str("] as const;\n\n");
}

fn render_ts_categories(buf: &mut String, ops: &[FrontendOperationSpec]) {
    let mut categories: Vec<&str> = ops.iter().map(|s| s.category).collect();
    categories.sort_unstable();
    categories.dedup();

    buf.push_str("export type OpCategory =\n");
    for (i, cat) in categories.iter().enumerate() {
        let sep = if i < categories.len() - 1 { "" } else { ";" };
        buf.push_str(&format!("  | {}{}\n", ts_string(cat), sep));
    }
    buf.push('\n');
}

fn render_ts_attr_constants(buf: &mut String) {
    buf.push_str("export const ATTR = {\n");
    for item in graph_attr_constants() {
        buf.push_str(&format!("  {}: {},\n", item.name, ts_string(item.value)));
    }
    buf.push_str("} as const;\n\n");
}

fn render_ts_graph_metric_constants(buf: &mut String) {
    buf.push_str("export const GRAPH_METRICS = {\n");
    for item in graph_metric_constants() {
        buf.push_str(&format!("  {}: {},\n", item.name, ts_string(item.value)));
    }
    buf.push_str("} as const;\n\n");
}

fn render_ts_provider_types(buf: &mut String) {
    let protocols = provider_protocols();
    buf.push_str("export type ProviderProtocol =\n");
    for (i, proto) in protocols.iter().enumerate() {
        let sep = if i < protocols.len() - 1 { "" } else { ";" };
        buf.push_str(&format!("  | {}{}\n", ts_string(proto), sep));
    }
    buf.push('\n');

    buf.push_str("export type ProviderSpec = {\n");
    buf.push_str("  readonly id: string;\n");
    buf.push_str("  readonly protocol: ProviderProtocol;\n");
    buf.push_str("  readonly defaultBaseUrl: string | null;\n");
    buf.push_str("  readonly requiresApiKey: boolean;\n");
    buf.push_str("  readonly apiKeyEnvVar: string | null;\n");
    buf.push_str("};\n\n");

    let providers = builtin_providers();
    buf.push_str("export const BUILTIN_PROVIDERS: readonly ProviderSpec[] = [\n");
    for p in &providers {
        buf.push_str("  {\n");
        buf.push_str(&format!("    id: {},\n", ts_string(p.id)));
        buf.push_str(&format!(
            "    protocol: {} as ProviderProtocol,\n",
            ts_string(p.protocol)
        ));
        buf.push_str(&format!(
            "    defaultBaseUrl: {},\n",
            ts_optional_string(p.default_base_url)
        ));
        buf.push_str(&format!(
            "    requiresApiKey: {},\n",
            ts_bool(p.requires_api_key)
        ));
        buf.push_str(&format!(
            "    apiKeyEnvVar: {},\n",
            ts_optional_string(p.api_key_env_var)
        ));
        buf.push_str("  },\n");
    }
    buf.push_str("] as const;\n\n");
}

fn ts_string(value: &str) -> String {
    serde_json::to_string(value).expect("ts string literal")
}

fn ts_optional_string(value: Option<&str>) -> String {
    match value {
        Some(v) => ts_string(v),
        None => "null".to_string(),
    }
}

fn ts_bool(value: bool) -> &'static str {
    if value { "true" } else { "false" }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generic_frontend_codegen_has_closed_contract_outputs() {
        let rendered = render_typescript_frontend_files();

        // The authoring surface exposes no operation constants, so this arm's
        // only generated frontend metadata is the runtime-evidence fact
        // binding. The capability catalogue that shares `src/generated` is a
        // bindable-name vocabulary, not an operation vocabulary, and is
        // rendered — and separately pinned — by `codegen_capabilities`.
        assert_eq!(
            rendered
                .iter()
                .map(|(filename, _)| *filename)
                .collect::<Vec<_>>(),
            vec![RUNTIME_EVIDENCE_TYPESCRIPT_FILE]
        );
        let evidence = &rendered[0].1;
        for required in [
            "LoopIterationCompletedFact",
            "NodeExecutionRecordedFact",
            "decodeFact",
            "runtimeSchema",
            "unknown enum value",
        ] {
            assert!(evidence.contains(required), "{required}");
        }
    }

    /// The generated TypeScript surface publishes no selection vocabulary.
    ///
    /// `generated.ts` is what every TypeScript consumer of this runtime's
    /// metadata imports, so a name admitted here is admitted on the authoring
    /// surface. A `model.call` target resolves to exactly one bound Model
    /// Deployment and a spawn names exactly one profile, so no generated
    /// constant, type, or table describes routing among candidates, ranking
    /// them, or reporting which were passed over.
    ///
    /// The assertion runs against freshly rendered output rather than the
    /// checked-in file, so it measures the generator, and it scans for
    /// selection markers rather than listing the emitters it forbids: adding
    /// an `ALL_AGENTS` table with `routeCapabilities`, or a routing attribute
    /// upstream in `ALL_ATTR_NAMES`, fails here without this test being
    /// edited.
    #[test]
    fn generated_typescript_publishes_no_selection_vocabulary() {
        let generated = render_generated_typescript();
        let selection_markers = [
            "routeCapabilities",
            "route_capabilities",
            "ROUTE_",
            "agent_route",
            "AGENT_ROUTE",
            "preferred_profiles",
            "PREFERRED_PROFILES",
            "eligible_profiles",
            "rejected_profiles",
        ];
        let offenders: Vec<&str> = selection_markers
            .into_iter()
            .filter(|marker| generated.contains(marker))
            .collect();
        assert!(
            offenders.is_empty(),
            "generated TypeScript publishes a selection vocabulary: {offenders:?}"
        );
    }
}
