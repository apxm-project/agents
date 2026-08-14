// The Agent definition factory and its compiled program handle.
//
// Agent(...) binds one typed program from a static source declaration and
// captures its complete callback AST into the FrontendGraph. The returned handle
// exposes new(...) and invoke(...) composition and the compiler bridge result.

import type { Json } from "./contract.js";
import { authoredSource } from "./authored-source.js";
import { compilerService } from "./compiler-service.js";
import { captureProgram, type ProgramBinding } from "./capture.js";
import { declaredSoFar, recordDeclaration } from "./declared.js";
import type { ContextSchema } from "./markers.js";
import { stableDigest } from "./markers.js";

type AgentCallback<Context, Input, Output> = {
  context: Context;
  yield_(output: Output): Promise<Input>;
};

/**
 * Everything an Agent states about itself.
 *
 * The typed interface is not here: it is `Agent<Input, Output, Context>`, read
 * back from the authored source. Neither are the declarations the body uses —
 * they are the ones the module declared — nor the source text, which the module
 * states once with `source(import.meta.url)`.
 */
type AgentConfig<Input, Output, Context> = {
  readonly name?: string;
  readonly context?: ContextSchema;
  run(agent: AgentCallback<Context, Input, Output>, input: Input): Promise<Output> | Output;
};

export class AgentHandle<Input, Output, Context> implements ProgramBinding {
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

  new(_options?: { context?: Context }): ProgramInstance<Input, Output, Context> {
    return new ProgramInstance<Input, Output, Context>(this.programId, this.artifactDigest);
  }

  invoke(_input: Input): Promise<Output> {
    throw new Error("Agent.invoke is called inside a compiled Agent body");
  }
}

export class ProgramInstance<Input, Output, Context> {
  constructor(
    readonly programRef: string,
    readonly artifactDigest: string,
  ) {}

  invoke(_input: Input): Promise<Output> {
    throw new Error("instance.invoke is called inside a compiled Agent body");
  }
}

export function Agent<Input, Output, Context = undefined>(
  config: AgentConfig<Input, Output, Context>,
): AgentHandle<Input, Output, Context> {
  const programId = config.name ?? (config.run.name || "Agent");
  const graph = captureProgram({
    programId,
    // Program identity is the entrypoint name in the source contract.
    entrypoint: programId,
    contextSchema: config.context,
    declared: declaredSoFar(),
    source: authoredSource(),
  });

  return recordDeclaration(new AgentHandle<Input, Output, Context>(programId, graph));
}
