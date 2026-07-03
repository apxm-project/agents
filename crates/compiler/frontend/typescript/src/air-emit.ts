/**
 * MLIR text emission helpers for AIS operations.
 *
 * Mirrors the shape produced by the Python frontend's generated
 * `apxm/_generated/emission.py` (per-op emitters keyed by opcode), but uses a
 * single spec-driven emitter instead of one hand-generated function per op:
 * the op-spec.v1 catalog (`OP_SPECS`) carries enough field metadata (ordered
 * field list, which fields are string-typed) to pick a positional "primary"
 * argument and render the rest as keyword attributes. Byte-identical parity
 * with the Python emitter is explicitly out of scope for this package
 * (tracked separately as TSF-4); this emitter targets grammar-valid,
 * semantically-equivalent `.air` text.
 */
import { OP_SPECS, VOID_OPS, type OpName } from "./generated/ops.js";

export function quote(value: string): string {
  const escaped = value
    .replace(/\\/g, "\\\\")
    .replace(/"/g, '\\"')
    .replace(/\n/g, "\\n")
    .replace(/\t/g, "\\t")
    .replace(/\r/g, "\\r");
  return `"${escaped}"`;
}

export function formatAttrValue(value: unknown): string {
  if (value === null || value === undefined) {
    return quote("null");
  }
  if (typeof value === "boolean") {
    return value ? "true" : "false";
  }
  if (typeof value === "number") {
    return Number.isInteger(value) ? `${value} : i64` : `${value} : f64`;
  }
  if (typeof value === "string") {
    return quote(value);
  }
  if (Array.isArray(value)) {
    return `[${value.map((v) => formatAttrValue(v)).join(", ")}]`;
  }
  if (typeof value === "object") {
    const record = value as Record<string, unknown>;
    const items = Object.keys(record)
      .sort()
      .map((key) => `${key} = ${formatAttrValue(record[key])}`);
    return `{${items.join(", ")}}`;
  }
  return quote(String(value));
}

/**
 * Emit MLIR assembly for a single AIS operation.
 *
 * `ssaName` is the `%name` this node's result binds to (ignored for void
 * ops). `attrs` is the node's attribute map; `inputs` is the ordered list of
 * already-emitted SSA names this op reads as Data operands.
 */
export function emitOp(
  op: OpName,
  ssaName: string,
  attrs: Record<string, unknown>,
  inputs: readonly string[],
): string {
  const spec = OP_SPECS[op];
  const lowered = op.toLowerCase();
  const isVoid = VOID_OPS.has(op);

  const ctx =
    inputs.length > 0
      ? ` [${inputs.join(", ")} : ${inputs.map(() => "!ais.token").join(", ")}]`
      : "";

  // Pick the first spec-declared field with a present string value as the
  // inline positional "primary" argument (mirrors e.g. `ais.ask "prompt"`).
  let primaryField: string | undefined;
  if (spec) {
    for (const field of spec.fields) {
      const value = attrs[field.name];
      if (typeof value === "string") {
        primaryField = field.name;
        break;
      }
    }
  }
  const primary = primaryField !== undefined ? ` ${quote(String(attrs[primaryField]))}` : "";

  const kwParts: string[] = [];
  for (const key of Object.keys(attrs).sort()) {
    if (key === primaryField) continue;
    const value = attrs[key];
    if (value === null || value === undefined) continue;
    kwParts.push(`${key} = ${formatAttrValue(value)}`);
  }
  const kwStr = kwParts.length > 0 ? ` {${kwParts.join(", ")}}` : "";

  const resultPrefix = isVoid ? "" : `${ssaName} = `;
  const typeSuffix = isVoid ? "" : " : !ais.token";

  return `${resultPrefix}ais.${lowered}${primary}${ctx}${kwStr}${typeSuffix}`;
}

export function isVoidOp(op: OpName): boolean {
  return VOID_OPS.has(op);
}
