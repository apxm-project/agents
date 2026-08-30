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

// Capture host intrinsics before evaluated source runs.  The public handle is
// consumed after source evaluation, so calls through mutable global JSON/Object
// properties would otherwise let source replace the clone/freeze operations.
const jsonParse = JSON.parse.bind(JSON);
const jsonStringify = JSON.stringify.bind(JSON);
const objectFreeze = Object.freeze.bind(Object);
const objectValues = Object.values.bind(Object);
const WeakSetConstructor = WeakSet;

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

/**
 * Keep captured graph data immutable while it crosses the evaluated-source
 * boundary.  A shallow `readonly` type is not a runtime security boundary:
 * submitted code can cast the graph to `any` and rewrite nested declarations
 * or requirements after capture.  The graph is JSON data, so recursively
 * freezing objects and arrays is both sufficient and deterministic.
 */
function freezeJson(value: Json): Json {
  const seen = new WeakSetConstructor<object>();

  const freeze = (current: unknown): void => {
    if (current === null || typeof current !== "object" || seen.has(current)) {
      return;
    }
    seen.add(current);
    for (const child of objectValues(current as Record<string, unknown>)) {
      freeze(child);
    }
    objectFreeze(current);
  };

  freeze(value);
  return value;
}

export class AgentHandle<Input, Output, Context> implements ProgramBinding {
  #graph: Json;
  readonly kind = "agent_definition" as const;
  readonly artifactDigest: string;
  readonly entrypoint: string;
  readonly targetAgentIdentityRequirement: string;

  constructor(
    readonly programId: string,
    graph: Json,
  ) {
    this.#graph = freezeJson(graph);
    this.artifactDigest = stableDigest(jsonStringify(this.#graph));
    this.entrypoint = programId;
    this.targetAgentIdentityRequirement = `${programId}.identity`;
    // `readonly` and `private` disappear at runtime. Freeze the handle as
    // well, so an `any` cast cannot replace its internal graph reference.
    objectFreeze(this);
  }

  frontendGraph(): Json {
    // Return a fresh value so source that inspects the graph cannot mutate the
    // value the capture harness requests after module evaluation.
    return jsonParse(jsonStringify(this.#graph)) as Json;
  }

  diagnostics(): string | null {
    return compilerService().verifyGraph(this.#graph);
  }

  canonicalAir(): string {
    return compilerService().canonicalAir(this.#graph);
  }

  artifact(): Json {
    return compilerService().artifact(this.#graph);
  }

  new(_options?: { context?: Context }): ProgramInstance<Input, Output, Context> {
    return new ProgramInstance<Input, Output, Context>(this.programId, this.artifactDigest);
  }

  invoke(_input: Input): Promise<Output> {
    throw new Error("Agent.invoke is called inside a compiled Agent body");
  }
}

// The evaluated source can otherwise replace a method on the shared
// prototype after an instance is created, bypassing the frozen instance.
objectFreeze(AgentHandle.prototype);

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
