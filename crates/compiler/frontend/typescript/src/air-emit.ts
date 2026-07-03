/**
 * MLIR text emission helpers for AIS operations.
 *
 * Mirrors the Python frontend's generated `apxm/_generated/emission.py`
 * (per-op emitters keyed by opcode) byte-for-byte for the op shapes each
 * frontend actually needs to agree on (TSF-4 parity lock). Each opcode has
 * its own syntactic quirks in the AIR grammar — which field is the inline
 * "primary" quoted argument, whether dependency operands render as
 * `[...]` or `(...)`, whether a field renders as `to "..."` syntactic sugar
 * instead of a `key = value` attribute, and so on — so this is a small
 * per-op rule table (`OP_EMIT_RULES`) rather than one fully generic
 * formatter, exactly like the Python side. Ops with no explicit rule fall
 * back to a generic OP_SPECS-driven formatter (grammar-valid, not
 * necessarily byte-identical to Python for those ops).
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

type CtxStyle = "bracket" | "paren" | "none";

interface OpEmitRule {
  /** Attribute rendered as the inline quoted positional argument, e.g. `ais.ask "prompt"`. */
  primaryField?: string;
  /** How Data-dependency operands render: `[a, b : !ais.token, !ais.token]`, `(a : !ais.token)`, or omitted entirely. */
  ctx: CtxStyle;
  /** `{field, word}` pairs rendered as ` <word> "<value>"` syntactic sugar (e.g. `to "worker"`), in order. */
  synKw?: Array<{ field: string; word: string }>;
  /** Attribute names rendered as fixed-order leading `key = value` pairs (if present), before the alphabetical remainder. */
  kwFirst?: string[];
  /** Attribute names consumed elsewhere (primary/synKw/positionalParen) and never rendered as a generic `key = value` pair. */
  excludeFromKw: string[];
  /** Attribute rendered as a quoted `(value)` positional group, e.g. INV_TOOL's `params_json`. */
  positionalParenField?: string;
  /** When true, attributes are ignored entirely for text purposes (op always emits bare `ais.op : !ais.token`). */
  ignoreAttrs?: boolean;
}

/**
 * Per-op emission rules, hand-mirrored from `apxm/_generated/emission.py`'s
 * per-op `emit_*` functions for every op the two frontends' shared AIR
 * vector fixtures (`workspace/contracts/vectors/air/*.air`) exercise.
 */
const OP_EMIT_RULES: Partial<Record<OpName, OpEmitRule>> = {
  ASK: {
    primaryField: "template_str",
    ctx: "bracket",
    kwFirst: ["temperature", "model", "provider", "backend", "system_prompt"],
    excludeFromKw: ["backend", "model", "provider", "system_prompt", "temperature", "template_str"],
  },
  THINK: {
    primaryField: "template_str",
    ctx: "bracket",
    kwFirst: ["budget", "temperature", "model", "provider", "backend", "system_prompt"],
    excludeFromKw: [
      "backend",
      "budget",
      "model",
      "provider",
      "system_prompt",
      "temperature",
      "template_str",
    ],
  },
  FENCE: {
    ctx: "bracket",
    ignoreAttrs: true,
    excludeFromKw: [],
  },
  RESUME: {
    ctx: "bracket",
    ignoreAttrs: true,
    excludeFromKw: [],
  },
  DELEGATE: {
    primaryField: "task_spec",
    ctx: "paren",
    synKw: [{ field: "target_agent", word: "to" }],
    excludeFromKw: ["target_agent", "task_spec"],
  },
  COMMUNICATE: {
    primaryField: "message",
    ctx: "paren",
    synKw: [{ field: "recipient", word: "to" }],
    kwFirst: ["protocol"],
    excludeFromKw: ["message", "protocol", "recipient"],
  },
  HANDOFF: {
    primaryField: "handoff_from",
    ctx: "paren",
    synKw: [{ field: "handoff_to", word: "to" }],
    kwFirst: ["transfer_state"],
    excludeFromKw: ["handoff_from", "handoff_to", "transfer_state"],
  },
  INV_TOOL: {
    primaryField: "capability",
    ctx: "bracket",
    positionalParenField: "params_json",
    excludeFromKw: ["capability", "params_json"],
  },
  SPAWN_AGENT: {
    primaryField: "agent_name",
    ctx: "none",
    kwFirst: [
      "profile",
      "agent_route",
      "required_capabilities",
      "preferred_profiles",
      "mode",
      "model",
      "cwd",
      "system_prompt",
    ],
    excludeFromKw: [
      "agent_name",
      "agent_route",
      "cwd",
      "mode",
      "model",
      "preferred_profiles",
      "profile",
      "required_capabilities",
      "system_prompt",
    ],
  },
  REGISTER_CAPABILITY: {
    primaryField: "capability_name",
    ctx: "none",
    kwFirst: ["description", "parameters_schema", "python_handler_id"],
    excludeFromKw: ["capability_name", "description", "parameters_schema", "python_handler_id"],
  },
  REGISTER_HOOK: {
    primaryField: "hook_event",
    ctx: "none",
    kwFirst: ["hook_match", "hook_mode", "python_hook_handler_id"],
    excludeFromKw: ["hook_event", "hook_match", "hook_mode", "python_hook_handler_id"],
  },
  AUTONOMOUS: {
    primaryField: "prompt",
    ctx: "paren",
    kwFirst: ["max_iterations", "backend", "model", "provider", "system_prompt", "temperature"],
    excludeFromKw: [
      "backend",
      "max_iterations",
      "model",
      "prompt",
      "provider",
      "system_prompt",
      "temperature",
    ],
  },
  CHECKPOINT: {
    primaryField: "checkpoint_id",
    ctx: "none",
    excludeFromKw: ["checkpoint_id"],
  },
  CONST_STR: {
    primaryField: "value",
    ctx: "none",
    excludeFromKw: ["value"],
  },
};

function ctxText(style: CtxStyle, inputs: readonly string[]): string {
  if (style === "none" || inputs.length === 0) return "";
  const types = inputs.map(() => "!ais.token").join(", ");
  return style === "bracket" ? ` [${inputs.join(", ")} : ${types}]` : ` (${inputs.join(", ")} : ${types})`;
}

function renderKwParts(attrs: Record<string, unknown>, kwFirst: string[], excludeFromKw: string[]): string[] {
  const kwParts: string[] = [];
  const consumed = new Set(excludeFromKw);
  for (const key of kwFirst) {
    const value = attrs[key];
    if (value !== null && value !== undefined) {
      kwParts.push(`${key} = ${formatAttrValue(value)}`);
    }
  }
  for (const key of Object.keys(attrs).sort()) {
    if (consumed.has(key) || kwFirst.includes(key)) continue;
    const value = attrs[key];
    if (value === null || value === undefined) continue;
    kwParts.push(`${key} = ${formatAttrValue(value)}`);
  }
  return kwParts;
}

function emitWithRule(rule: OpEmitRule, op: OpName, ssaName: string, attrs: Record<string, unknown>, inputs: readonly string[]): string {
  const lowered = op.toLowerCase();
  const ctx = ctxText(rule.ctx, inputs);

  if (rule.ignoreAttrs) {
    return `${ssaName} = ais.${lowered}${ctx} : !ais.token`;
  }

  const primary =
    rule.primaryField !== undefined && typeof attrs[rule.primaryField] === "string"
      ? ` ${quote(String(attrs[rule.primaryField]))}`
      : "";

  const positionalParen =
    rule.positionalParenField !== undefined
      ? ` (${quote(String(attrs[rule.positionalParenField] ?? "{}"))})`
      : "";

  const synKw = (rule.synKw ?? [])
    .map(({ field, word }) => {
      const value = attrs[field];
      return value !== null && value !== undefined ? ` ${word} ${quote(String(value))}` : "";
    })
    .join("");

  const kwParts = renderKwParts(attrs, rule.kwFirst ?? [], rule.excludeFromKw);
  const kwStr = kwParts.length > 0 ? ` {${kwParts.join(", ")}}` : "";

  return `${ssaName} = ais.${lowered}${primary}${positionalParen}${synKw}${ctx}${kwStr} : !ais.token`;
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
  const rule = OP_EMIT_RULES[op];
  if (rule) {
    return emitWithRule(rule, op, ssaName, attrs, inputs);
  }

  // Generic OP_SPECS-driven fallback for ops without a hand-mirrored rule:
  // grammar-valid, not guaranteed byte-identical to the Python emitter.
  const spec = OP_SPECS[op];
  const lowered = op.toLowerCase();
  const isVoid = VOID_OPS.has(op);

  const ctx =
    inputs.length > 0
      ? ` [${inputs.join(", ")} : ${inputs.map(() => "!ais.token").join(", ")}]`
      : "";

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
  if (OP_EMIT_RULES[op]) {
    // Every hand-mirrored rule above targets an op that always produces a
    // `!ais.token` result in its AIR text (matches Python's per-op emit_*
    // templates, which is independent of op-spec's `produces_output` flag —
    // e.g. FENCE is declared non-producing in the catalog but its emitter
    // always renders `%x = ais.fence ... : !ais.token`).
    return false;
  }
  return VOID_OPS.has(op);
}
