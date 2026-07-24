// Public authoring markers for typed Agent source.
//
// These are compile-time declarations resolved by imported symbol identity and
// type. They declare schemas, bindings, and handlers; they never execute the
// authored body to discover the graph, and they carry no credential, grant,
// endpoint, or runtime object.

const FORBIDDEN_DISPLAY_NAMES = new Set(["default", "support", "search-web", ""]);

function rejectDisplayName(value: string, marker: string): void {
  if (FORBIDDEN_DISPLAY_NAMES.has(value)) {
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
  readonly kind: "tool_binding" | "tool_handler";
  readonly targetRef: string;
  readonly inputTypeRef: string;
  readonly outputTypeRef: string;
  readonly handlerDigest?: string;
  (args: I): Promise<O>;
};

export type CapabilityBinding<I = unknown, O = unknown> = {
  readonly kind: "capability_binding" | "capability_handler";
  readonly targetRef: string;
  readonly inputTypeRef: string;
  readonly outputTypeRef: string;
  readonly handlerDigest?: string;
  (args: I): Promise<O>;
};

export type EventTypeBinding<T = unknown> = {
  readonly kind: "event_type";
  readonly typeRef: string;
  wait(): Promise<T>;
};

export type ContextSchema = {
  readonly kind: "context";
  readonly typeRef: string;
  readonly defaultPresent: boolean;
};

export type HandlerSpec<I, O> = {
  run(input: I): Promise<O> | O;
};

function digestOf(fn: (...args: never[]) => unknown): string {
  const source = fn.toString();
  let hash = 5381;
  for (let index = 0; index < source.length; index += 1) {
    hash = ((hash << 5) + hash + source.charCodeAt(index)) & 0xffffffff;
  }
  return `sha256:${(hash >>> 0).toString(16).padStart(64, "0")}`;
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

export function Tool<I, O>(target: string | HandlerSpec<I, O>): ToolBinding<I, O> {
  const binding = () => uncallable("Tool");
  if (typeof target === "string") {
    rejectDisplayName(target, "Tool");
    return Object.assign(binding, {
      kind: "tool_binding" as const,
      targetRef: target,
      inputTypeRef: "ToolInput",
      outputTypeRef: "ToolOutput",
    });
  }
  return Object.assign(binding, {
    kind: "tool_handler" as const,
    targetRef: `tool:${target.run.name || "handler"}`,
    inputTypeRef: "ToolInput",
    outputTypeRef: "ToolOutput",
    handlerDigest: digestOf(target.run as (...args: never[]) => unknown),
  });
}

export function Capability<I, O>(
  target: string | HandlerSpec<I, O>,
): CapabilityBinding<I, O> {
  const binding = () => uncallable("Capability");
  if (typeof target === "string") {
    rejectDisplayName(target, "Capability");
    return Object.assign(binding, {
      kind: "capability_binding" as const,
      targetRef: target,
      inputTypeRef: "CapabilityInput",
      outputTypeRef: "CapabilityOutput",
    });
  }
  return Object.assign(binding, {
    kind: "capability_handler" as const,
    targetRef: `capability:${target.run.name || "handler"}`,
    inputTypeRef: "CapabilityInput",
    outputTypeRef: "CapabilityOutput",
    handlerDigest: digestOf(target.run as (...args: never[]) => unknown),
  });
}

export function Event<T>(typeRef = "Event"): EventTypeBinding<T> {
  return {
    kind: "event_type",
    typeRef,
    wait: () => uncallable("Event"),
  };
}

export function Context<C>(initial?: C): ContextSchema {
  return {
    kind: "context",
    typeRef: "Context",
    defaultPresent: initial !== undefined,
  };
}
