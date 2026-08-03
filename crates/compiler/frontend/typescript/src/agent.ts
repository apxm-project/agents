// The Agent definition factory and its compiled program handle.
//
// Agent(...) binds one typed program from a static source declaration and
// captures its complete callback AST into the FrontendGraph. The returned handle
// exposes new(...) and invoke(...) composition and the compiler bridge result.

import type { Json } from "./contract.js";
import { compilerService } from "./compiler-service.js";
import {
  captureProgram,
  type Binding,
  type ProgramBinding,
  type StaticSource,
} from "./capture.js";
import type { ContextSchema } from "./markers.js";
import { stableDigest } from "./markers.js";

type AgentCallback<C, I, O> = {
  context: C;
  yield_(output: O): Promise<I>;
};

type AgentConfig<I, O, C> = {
  readonly name?: string;
  /** Closed input type-ref identity; defaults to `Input` when omitted. */
  readonly input?: string;
  /** Closed output type-ref identity; defaults to `Output` when omitted. */
  readonly output?: string;
  readonly context?: ContextSchema;
  readonly use?: Readonly<Record<string, Binding>>;
  readonly source?: StaticSource;
  run(agent: AgentCallback<C, I, O>, input: I): Promise<O> | O;
};

export class AgentHandle<I, O, C> implements ProgramBinding {
  readonly kind = "agent_definition" as const;
  readonly artifactDigest: string;
  readonly entrypoint: string;
  readonly targetAgentIdentityRequirement: string;

  constructor(
    readonly programId: string,
    private readonly graph: Json,
  ) {
    this.artifactDigest = stableDigest(JSON.stringify(graph));
    this.entrypoint = programId;
    this.targetAgentIdentityRequirement = `${programId}.identity`;
  }

  frontendGraph(): Json {
    return this.graph;
  }

  diagnostics(): string | null {
    return compilerService().verifyGraph(this.graph);
  }

  canonicalAir(): string {
    return compilerService().canonicalAir(this.graph);
  }

  artifact(): Json {
    return compilerService().artifact(this.graph);
  }

  new(_options?: { context?: C }): ProgramInstance<I, O, C> {
    return new ProgramInstance<I, O, C>(this.programId, this.artifactDigest);
  }

  invoke(_input: I): Promise<O> {
    throw new Error("Agent.invoke is called inside a compiled Agent body");
  }
}

export class ProgramInstance<I, O, C> {
  constructor(
    readonly programRef: string,
    readonly artifactDigest: string,
  ) {}

  invoke(_input: I): Promise<O> {
    throw new Error("instance.invoke is called inside a compiled Agent body");
  }
}

export function Agent<I, O, C = undefined>(
  config: AgentConfig<I, O, C>,
): AgentHandle<I, O, C> {
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
    bindingDeclIds.set("__context__", `decl.context.${contextTypeRef}`);
  }

  const graph = captureProgram({
    programId,
    // Program identity is the entrypoint name in the source contract.
    entrypoint: programId,
    inputTypeRef: config.input ?? "Input",
    outputTypeRef: config.output ?? "Output",
    contextTypeRef,
    hasDefaultContext,
    bindings,
    bindingDeclIds,
    source: config.source ?? missingStaticSource(),
  });

  return new AgentHandle<I, O, C>(programId, graph);
}

function declId(name: string, binding: Binding): string {
  switch (binding.kind) {
    case "model_binding":
      return `decl.model.${name}`;
    case "tool_binding":
      return `decl.tool.${name}`;
    case "capability_binding":
      return `decl.capability.${name}`;
    case "event_type":
      return `decl.event.${name}`;
    case "context":
      return `decl.context.${name}`;
    case "agent_definition":
      return binding.programId;
  }
}

function missingStaticSource(): never {
  throw new Error("Agent requires a static source token from its compiler bridge");
}
