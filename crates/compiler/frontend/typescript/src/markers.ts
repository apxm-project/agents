// Public authoring markers for typed Agent source.
//
// These are compile-time declarations resolved by imported symbol identity and
// type. They declare schemas, bindings, and handlers; they never execute the
// authored body to discover the graph, and they carry no credential, grant,
// endpoint, or runtime object.

import { recordDeclaration } from "./declared.js";
import {
  CAPABILITY_DISPLAY_NAME_REJECTED,
  CAPABILITY_REF_NOT_EXACT,
  type DiagnosticCode,
  EVENT_NOT_TYPED,
  EVENT_WAIT_OUTSIDE_BODY,
  MODEL_DISPLAY_NAME_REJECTED,
  SKILL_ENTRY_PATH_NOT_CANONICAL,
  SKILL_ID_NOT_EXACT,
  SKILL_INSTRUCTIONS_OVERLONG,
  SKILL_LOAD_OUTSIDE_BODY,
  SKILL_SOURCE_AMBIGUOUS,
  SKILL_SOURCE_MISSING,
  TOOL_DISPLAY_NAME_REJECTED,
  TOOL_REF_NOT_CAPABILITY,
} from "./generated/diagnostics.js";
import {
  SKILL_INSTRUCTION_KIND_ENTRY,
  SKILL_INSTRUCTION_KIND_INLINE,
} from "./generated/frontend-graph.js";
import type { SkillInstructionSource } from "./generated/frontend-records.js";
import {
  PERMISSION_DECISIONS,
  type Permission,
} from "./generated/permissions.js";
// The generated catalogue's `CapabilityId` is the closed set of builtin ids; the
// packaging surface's `CapabilityId` is the object a handler declaration hands
// back. Two different concepts wearing one name, so each is renamed to the thing
// it actually is at the point where both meet.
import type { CapabilityId as BuiltinCapabilityId } from "./generated/capabilities.js";
import { BUILTIN_CAPABILITIES } from "./generated/capabilities.js";

// Capture the intrinsic before authored code can replace the global method.
// Marker records are inputs to static capture, so their fields must remain
// stable after the declaration factory returns.
const objectFreeze = Object.freeze.bind(Object);

/**
 * The ceiling the skill-reading capability enforces on a body it loads. An
 * inline skill is the same trusted context landing in the same model window, so
 * it is refused here rather than at execution.
 */
const MAX_INSTRUCTION_BYTES = 128 * 1024;

const FORBIDDEN_DISPLAY_NAMES = new Set(["default", "model.default", "support", "search-web", ""]);

/**
 * The reference a shipped handler declaration hands back. The id and the
 * implementation arrive as one object, so there is no second place to spell the
 * id and nothing for the two spellings to disagree about.
 */
export type ShippedCapabilityReference = { readonly capabilityId: string };

/**
 * A Capability reference is a builtin id from the generated catalogue or the
 * handler that implements one. Nothing else: an invented bare string is not a
 * member of either arm, so referencing a Capability nobody implements is a type
 * error rather than a check some later pass has to remember to run.
 */
export type CapabilityReference = BuiltinCapabilityId | ShippedCapabilityReference;

function capabilityIdOf(reference: CapabilityReference): string {
  return typeof reference === "string" ? reference : reference?.capabilityId;
}

function rejectDisplayName(value: unknown, marker: string, code: DiagnosticCode): void {
  if (typeof value !== "string" || FORBIDDEN_DISPLAY_NAMES.has(value)) {
    throw new Error(
      `${code}: ${marker} accepts an exact typed reference, not a display name '${value}'`,
    );
  }
}

/**
 * Refuse a Capability reference that neither catalogue nor package mints.
 *
 * The type union already settles this for TypeScript source. JavaScript source
 * is authored against the same surface with no type-checker between it and the
 * frontend, so the closed set is settled here too: a bare string is admitted
 * only when the generated catalogue mints it, and any other reference has to be
 * the object a handler declaration handed back.
 */
function rejectUnmintedCapability(
  reference: CapabilityReference,
  marker: string,
  code: DiagnosticCode,
): void {
  if (typeof reference !== "string") {
    return;
  }
  if (!(BUILTIN_CAPABILITIES as readonly string[]).includes(reference)) {
    throw new Error(
      `${code}: ${marker} accepts a builtin catalogue id or the handler ` +
        `declaration that implements one, not '${reference}'`,
    );
  }
}

// A binding is authored as a call inside the Agent body; at the type level it is
// a callable so ordinary source type-checks, while the frontend resolves the call
// statically rather than executing it.
export type ModelBinding<I = unknown, O = unknown> = {
  readonly kind: "model_binding";
  readonly targetRef: string;
  readonly inputTypeRef: string;
  readonly outputTypeRef: string;
  (request: I): Promise<O>;
};

export type ToolBinding<I = unknown, O = unknown> = {
  readonly kind: "tool_binding";
  readonly targetRef: string;
  readonly inputTypeRef: string;
  readonly outputTypeRef: string;
  readonly permission?: Permission;
  (args: I): Promise<O>;
};

export type CapabilityBinding<I = unknown, O = unknown> = {
  readonly kind: "capability_binding";
  readonly targetRef: string;
  readonly inputTypeRef: string;
  readonly outputTypeRef: string;
  readonly permission?: Permission;
  (args: I): Promise<O>;
};

export type EventTypeBinding<T = unknown> = {
  readonly kind: "event_type";
  readonly typeRef: string;
  readonly targetRef: string;
  wait(): Promise<T>;
};

export type ContextSchema = {
  readonly kind: "context";
  readonly typeRef: string;
  readonly defaultPresent: boolean;
};

/**
 * Where a declared skill's instructions live. Exactly one field is stated: the
 * two are the two routes an edit takes to the artifact digest, and stating both
 * is a contradiction about where the instructions are.
 */
export type SkillSource = {
  readonly entry?: string;
  readonly text?: string;
};

/**
 * A declared Agent Skill. Loading one is an ordinary Capability invocation, not
 * a construct of its own: `await skill.load()` records a `capability.invoke` on
 * the skill-reading capability, so the authority to read the instructions is
 * declared in the artifact like any other.
 */
export type SkillBinding = {
  readonly kind: "skill";
  readonly skillId: string;
  readonly instructionSource: SkillInstructionSource;
  load(): Promise<string>;
};

/** The one package path the folder contract recognizes for this skill. */
function skillEntryPath(skillId: string): string {
  return `skills/${skillId}/SKILL.md`;
}

/** Derive a deterministic source-identity digest for static declarations. */
export function stableDigest(value: string): string {
  const bytes = new TextEncoder().encode(value);
  const paddedLength = ((bytes.length + 9 + 63) >> 6) << 6;
  const padded = new Uint8Array(paddedLength);
  padded.set(bytes);
  padded[bytes.length] = 0x80;
  const bitLength = BigInt(bytes.length) * 8n;
  for (let index = 0; index < 8; index += 1) {
    padded[paddedLength - 1 - index] = Number((bitLength >> BigInt(index * 8)) & 0xffn);
  }

  const state = [
    0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a,
    0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19,
  ];
  const words = new Uint32Array(64);
  for (let offset = 0; offset < padded.length; offset += 64) {
    for (let index = 0; index < 16; index += 1) {
      const position = offset + index * 4;
      words[index] = (
        (padded[position] << 24) |
        (padded[position + 1] << 16) |
        (padded[position + 2] << 8) |
        padded[position + 3]
      ) >>> 0;
    }
    for (let index = 16; index < 64; index += 1) {
      const first = words[index - 15];
      const second = words[index - 2];
      const smallSigma0 = rotateRight(first, 7) ^ rotateRight(first, 18) ^ (first >>> 3);
      const smallSigma1 = rotateRight(second, 17) ^ rotateRight(second, 19) ^ (second >>> 10);
      words[index] = (words[index - 16] + smallSigma0 + words[index - 7] + smallSigma1) >>> 0;
    }

    let [a, b, c, d, e, f, g, h] = state;
    for (let index = 0; index < 64; index += 1) {
      const bigSigma1 = rotateRight(e, 6) ^ rotateRight(e, 11) ^ rotateRight(e, 25);
      const choice = (e & f) ^ (~e & g);
      const temp1 = (h + bigSigma1 + choice + SHA256_ROUND_CONSTANTS[index] + words[index]) >>> 0;
      const bigSigma0 = rotateRight(a, 2) ^ rotateRight(a, 13) ^ rotateRight(a, 22);
      const majority = (a & b) ^ (a & c) ^ (b & c);
      const temp2 = (bigSigma0 + majority) >>> 0;
      h = g;
      g = f;
      f = e;
      e = (d + temp1) >>> 0;
      d = c;
      c = b;
      b = a;
      a = (temp1 + temp2) >>> 0;
    }
    state[0] = (state[0] + a) >>> 0;
    state[1] = (state[1] + b) >>> 0;
    state[2] = (state[2] + c) >>> 0;
    state[3] = (state[3] + d) >>> 0;
    state[4] = (state[4] + e) >>> 0;
    state[5] = (state[5] + f) >>> 0;
    state[6] = (state[6] + g) >>> 0;
    state[7] = (state[7] + h) >>> 0;
  }
  return `sha256:${state.map((word) => word.toString(16).padStart(8, "0")).join("")}`;
}

function rotateRight(value: number, amount: number): number {
  return (value >>> amount) | (value << (32 - amount));
}

const SHA256_ROUND_CONSTANTS = [
  0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5,
  0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174,
  0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
  0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967,
  0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85,
  0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
  0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
  0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
];

function uncallable(marker: string): never {
  throw new Error(`a ${marker} is invoked inside a compiled Agent body`);
}

/** Keep one declaration record immutable after it enters the module registry. */
function freezeDeclaration<T extends object>(declaration: T): T {
  return objectFreeze(declaration);
}

/** Copy a permission into an immutable plain record before capture can read it. */
function freezePermission(permission: Permission | undefined): Permission | undefined {
  if (permission === undefined) {
    return undefined;
  }
  const snapshot: Permission = {
    decision: permission.decision,
    ...(permission.reason === undefined ? {} : { reason: permission.reason }),
  };
  return freezeDeclaration(snapshot);
}

export function Model<Input, Output>(ref: string): ModelBinding<Input, Output> {
  rejectDisplayName(ref, "Model", MODEL_DISPLAY_NAME_REJECTED);
  const binding = () => uncallable("Model");
  return recordDeclaration(freezeDeclaration(Object.assign(binding, {
    kind: "model_binding" as const,
    targetRef: ref,
    inputTypeRef: "ModelRequest",
    outputTypeRef: "ModelResponse",
  })));
}

export function Tool<Input, Output>(
  capabilityRef: CapabilityReference,
  options: { readonly permission?: Permission } = {},
): ToolBinding<Input, Output> {
  const binding = () => uncallable("Tool");
  const targetRef = capabilityIdOf(capabilityRef);
  rejectDisplayName(targetRef, "Tool", TOOL_DISPLAY_NAME_REJECTED);
  rejectUnmintedCapability(capabilityRef, "Tool", TOOL_REF_NOT_CAPABILITY);
  rejectInvalidPermission(options.permission, "Tool");
  return recordDeclaration(freezeDeclaration(Object.assign(binding, {
    kind: "tool_binding" as const,
    targetRef,
    inputTypeRef: "ToolInput",
    outputTypeRef: "ToolOutput",
    permission: freezePermission(options.permission),
  })));
}

export function Capability<Input, Output>(
  ref: CapabilityReference,
  options: { readonly permission?: Permission } = {},
): CapabilityBinding<Input, Output> {
  const binding = () => uncallable("Capability");
  const targetRef = capabilityIdOf(ref);
  rejectDisplayName(targetRef, "Capability", CAPABILITY_DISPLAY_NAME_REJECTED);
  rejectUnmintedCapability(ref, "Capability", CAPABILITY_REF_NOT_EXACT);
  rejectInvalidPermission(options.permission, "Capability");
  return recordDeclaration(freezeDeclaration(Object.assign(binding, {
    kind: "capability_binding" as const,
    targetRef,
    inputTypeRef: "CapabilityInput",
    outputTypeRef: "CapabilityOutput",
    permission: freezePermission(options.permission),
  })));
}

/** Keep JavaScript callers on the generated closed permission vocabulary. */
function rejectInvalidPermission(
  permission: Permission | undefined,
  marker: string,
): void {
  if (permission === undefined) {
    return;
  }
  const candidate = permission as unknown as {
    decision?: unknown;
    reason?: unknown;
  };
  const decision = candidate?.decision;
  if (
    (typeof permission !== "object" && typeof permission !== "function") ||
    permission === null ||
    typeof decision !== "string" ||
    !(PERMISSION_DECISIONS as readonly string[]).includes(decision)
  ) {
    throw new TypeError(
      `${marker} permission is one of the generated Allow, Ask, or Deny markers`,
    );
  }
  if (
    candidate.reason !== undefined &&
    (typeof candidate.reason !== "string" || candidate.reason.length === 0)
  ) {
    throw new TypeError(`${marker} permission reason must be a non-empty string`);
  }
}

export function Event<Payload>(ref: string): EventTypeBinding<Payload> {
  rejectDisplayName(ref, "Event", EVENT_NOT_TYPED);
  return recordDeclaration(freezeDeclaration({
    kind: "event_type" as const,
    typeRef: "Event",
    targetRef: ref,
    wait: (): never => {
      throw new Error(
        `${EVENT_WAIT_OUTSIDE_BODY}: an Event is awaited inside a compiled Agent body`,
      );
    },
  }));
}

/**
 * Declare one Agent Skill, its instructions carried one of two ways.
 *
 * `entry` names the package file holding them, which the carrying package's
 * integrity chain hashes. `text` writes them here, inside the source bundle the
 * artifact digest already covers. Exactly one is stated.
 */
export function Skill(skillId: string, source: SkillSource): SkillBinding {
  if (typeof skillId !== "string" || FORBIDDEN_DISPLAY_NAMES.has(skillId)) {
    throw new Error(
      `${SKILL_ID_NOT_EXACT}: Skill accepts an exact typed reference, not a display name '${skillId}'`,
    );
  }
  const entry = source?.entry;
  const text = source?.text;
  if (entry !== undefined && text !== undefined) {
    throw new Error(
      `${SKILL_SOURCE_AMBIGUOUS}: Skill '${skillId}' states both a package entry and inline text; instructions live in one place`,
    );
  }
  if (entry === undefined && text === undefined) {
    throw new Error(
      `${SKILL_SOURCE_MISSING}: Skill '${skillId}' states neither a package entry nor inline text`,
    );
  }
  let instructionSource: SkillInstructionSource;
  if (text !== undefined) {
    if (text === "" || new TextEncoder().encode(text).length > MAX_INSTRUCTION_BYTES) {
      throw new Error(
        `${SKILL_INSTRUCTIONS_OVERLONG}: Skill '${skillId}' states an empty or oversized body; a loaded skill is at most ${MAX_INSTRUCTION_BYTES} bytes`,
      );
    }
    instructionSource = freezeDeclaration({ kind: SKILL_INSTRUCTION_KIND_INLINE, text });
  } else {
    const expected = skillEntryPath(skillId);
    if (entry !== expected) {
      throw new Error(
        `${SKILL_ENTRY_PATH_NOT_CANONICAL}: Skill '${skillId}' carries its instructions at '${expected}', not '${entry}'`,
      );
    }
    instructionSource = freezeDeclaration({ kind: SKILL_INSTRUCTION_KIND_ENTRY, path: expected });
  }
  return recordDeclaration(freezeDeclaration({
    kind: "skill" as const,
    skillId,
    instructionSource,
    load: (): never => {
      throw new Error(
        `${SKILL_LOAD_OUTSIDE_BODY}: a Skill is loaded inside a compiled Agent body`,
      );
    },
  }));
}

export function Context<Schema>(initial?: Schema): ContextSchema {
  return recordDeclaration(freezeDeclaration({
    kind: "context" as const,
    // The Context type identity is read off `Context<Schema>` in the authored
    // source, exactly as Python reads it off the decorated class.
    typeRef: "Context",
    defaultPresent: initial !== undefined,
  }));
}
