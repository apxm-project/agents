// Public authoring markers for typed Agent source.
//
// These are compile-time declarations resolved by imported symbol identity and
// type. They declare schemas, bindings, and handlers; they never execute the
// authored body to discover the graph, and they carry no credential, grant,
// endpoint, or runtime object.

const FORBIDDEN_DISPLAY_NAMES = new Set(["default", "model.default", "support", "search-web", ""]);

function rejectDisplayName(value: unknown, marker: string): void {
  if (typeof value !== "string" || FORBIDDEN_DISPLAY_NAMES.has(value)) {
    throw new Error(
      `${marker} accepts an exact typed reference, not a display name '${value}'`,
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
  readonly requestedPermission?: string;
  (args: I): Promise<O>;
};

export type CapabilityBinding<I = unknown, O = unknown> = {
  readonly kind: "capability_binding";
  readonly targetRef: string;
  readonly inputTypeRef: string;
  readonly outputTypeRef: string;
  readonly requestedPermission?: string;
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

export function Model<I, O>(ref: string): ModelBinding<I, O> {
  rejectDisplayName(ref, "Model");
  const binding = () => uncallable("Model");
  return Object.assign(binding, {
    kind: "model_binding" as const,
    targetRef: ref,
    inputTypeRef: "ModelRequest",
    outputTypeRef: "ModelResponse",
  });
}

export function Tool<I, O>(
  targetRef: string,
  options: { readonly requestedPermission?: string } = {},
): ToolBinding<I, O> {
  const binding = () => uncallable("Tool");
  rejectDisplayName(targetRef, "Tool");
  return Object.assign(binding, {
    kind: "tool_binding" as const,
    targetRef,
    inputTypeRef: "ToolInput",
    outputTypeRef: "ToolOutput",
    requestedPermission: options.requestedPermission,
  });
}

export function Capability<I, O>(
  targetRef: string,
  options: { readonly requestedPermission?: string } = {},
): CapabilityBinding<I, O> {
  const binding = () => uncallable("Capability");
  rejectDisplayName(targetRef, "Capability");
  return Object.assign(binding, {
    kind: "capability_binding" as const,
    targetRef,
    inputTypeRef: "CapabilityInput",
    outputTypeRef: "CapabilityOutput",
    requestedPermission: options.requestedPermission,
  });
}

export function Event<T>(targetRef: string, typeRef = "Event"): EventTypeBinding<T> {
  rejectDisplayName(targetRef, "Event");
  return {
    kind: "event_type",
    typeRef,
    targetRef,
    wait: () => uncallable("Event"),
  };
}

export function Context<C>(initial?: C, typeRef = "Context"): ContextSchema {
  return {
    kind: "context",
    typeRef,
    defaultPresent: initial !== undefined,
  };
}
