// The Agent definition factory and its compiled program handle.
//
// Agent(...) binds one typed program from a run callback and captures its source
// into the FrontendGraph on definition. The returned handle exposes new(...) and
// invoke(...) composition and the compiled graph for the compiler bridge.

import {
  canonicalAirJson,
  compileArtifact,
  verifyGraph,
  type Json,
} from "./bridge.js";
import { captureProgram, type Binding } from "./capture.js";
import type { ContextSchema } from "./markers.js";

export type AgentConfig<I, O, C> = {
  readonly name?: string;
  readonly context?: ContextSchema;
  readonly use?: Readonly<Record<string, Binding>>;
  run(agent: AgentFacade<C>, input: I): Promise<O> | O;
};

export type AgentFacade<C> = {
  context: C;
  yield_(output: unknown): Promise<unknown>;
};

export class AgentDefinition<I, O, C> {
  constructor(
    readonly programId: string,
    private readonly graph: Json,
  ) {}

  frontendGraph(): Json {
    return this.graph;
  }

  diagnostics(): string | null {
    return verifyGraph(this.graph);
  }

  canonicalAir(): string {
    return canonicalAirJson(this.graph);
  }

  artifact(): Json {
    return compileArtifact(this.graph);
  }

  new(_options?: { context?: C }): ProgramInstance<I, O, C> {
    return new ProgramInstance<I, O, C>(this.programId);
  }

  invoke(_input: I): Promise<O> {
    throw new Error("Agent.invoke is called inside a compiled Agent body");
  }
}

export class ProgramInstance<I, O, C> {
  constructor(readonly programRef: string) {}

  invoke(_input: I): Promise<O> {
    throw new Error("instance.invoke is called inside a compiled Agent body");
  }
}

export function Agent<I, O, C = undefined>(
  config: AgentConfig<I, O, C>,
): AgentDefinition<I, O, C> {
  const programId = config.name ?? (config.run.name || "Agent");

  const bindings = new Map<string, Binding>();
  const bindingDeclIds = new Map<string, string>();
  const use = config.use ?? {};
  for (const [name, binding] of Object.entries(use)) {
    bindings.set(name, binding);
    bindingDeclIds.set(name, declId(name, binding));
  }
  let contextTypeRef: string | undefined;
  let hasDefaultContext = false;
  if (config.context !== undefined) {
    contextTypeRef = config.context.typeRef;
    hasDefaultContext = config.context.defaultPresent;
    bindings.set("__context__", config.context);
    bindingDeclIds.set("__context__", `decl.context.__context__`);
  }

  const graph = captureProgram({
    programId,
    entrypoint: "run",
    inputTypeRef: "Input",
    outputTypeRef: "Output",
    contextTypeRef,
    hasDefaultContext,
    bindings,
    bindingDeclIds,
    callbackSource: config.run.toString(),
  });

  return new AgentDefinition<I, O, C>(programId, graph);
}

function declId(name: string, binding: Binding): string {
  switch (binding.kind) {
    case "model_binding":
      return `decl.model.${name}`;
    case "tool_binding":
    case "tool_handler":
      return `decl.tool.${name}`;
    case "capability_binding":
    case "capability_handler":
      return `decl.capability.${name}`;
    case "event_type":
      return `decl.event.${name}`;
    case "context":
      return `decl.context.${name}`;
  }
}
