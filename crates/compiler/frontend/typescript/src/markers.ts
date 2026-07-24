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
  (args: I): Promise<O>;
};

export type CapabilityBinding<I = unknown, O = unknown> = {
  readonly kind: "capability_binding";
  readonly targetRef: string;
  readonly inputTypeRef: string;
  readonly outputTypeRef: string;
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
  let hash = 5381;
  for (let index = 0; index < value.length; index += 1) {
    hash = ((hash << 5) + hash + value.charCodeAt(index)) & 0xffffffff;
  }
  return `sha256:${String(hash >>> 0).padStart(64, "0")}`;
}

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

export function Tool<I, O>(targetRef: string): ToolBinding<I, O> {
  const binding = () => uncallable("Tool");
  rejectDisplayName(targetRef, "Tool");
  return Object.assign(binding, {
    kind: "tool_binding" as const,
    targetRef,
    inputTypeRef: "ToolInput",
    outputTypeRef: "ToolOutput",
  });
}

export function Capability<I, O>(targetRef: string): CapabilityBinding<I, O> {
  const binding = () => uncallable("Capability");
  rejectDisplayName(targetRef, "Capability");
  return Object.assign(binding, {
    kind: "capability_binding" as const,
    targetRef,
    inputTypeRef: "CapabilityInput",
    outputTypeRef: "CapabilityOutput",
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
