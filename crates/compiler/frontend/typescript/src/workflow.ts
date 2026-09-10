// One sealed typed Program contract beneath Workflow and Agent declarations.

import type { Json } from "./contract.js";
import type { ProgramBinding } from "./capture.js";
import { captureProgram } from "./capture.js";
import { authoredSource } from "./authored-source.js";
import { declaredSoFar, recordDeclaration } from "./declared.js";
import { compilerService } from "./compiler-service.js";
import { stableDigest } from "./markers.js";
import type { ContextSchema, ModelBinding } from "./markers.js";

// Capture host intrinsics before evaluated source runs.  The public handle is
// consumed after source evaluation, so calls through mutable global JSON/Object
// properties would otherwise let source replace the clone/freeze operations.
const jsonParse = JSON.parse.bind(JSON);
const jsonStringify = JSON.stringify.bind(JSON);
const objectFreeze = Object.freeze.bind(Object);
const objectValues = Object.values.bind(Object);
const WeakSetConstructor = WeakSet;

declare const workflowBrand: unique symbol;

/** A typed compiled program, including Context, loops, Hooks and yield/resume. */
export interface Program<Input, Output, Context = undefined> extends ProgramBinding {
  readonly [workflowBrand]: true;
  frontendGraph(): Json;
  diagnostics(): string | null;
  canonicalAir(): string;
  artifact(): Json;
  readonly new: (options?: { context?: Context }) => ProgramInstance<Input, Output, Context>;
  readonly invoke: (input: Input) => Promise<Output>;
}

export type ProgramConfig<Input, Output, Context> = {
  readonly name?: string;
  readonly context?: ContextSchema;
  run(agent: { context: Context; yield_(output: Output): Promise<Input> }, input: Input): Promise<Output> | Output;
};

/** General orchestration: Model calls are optional, never implicitly provided. */
export function Workflow<Input, Output, Context = undefined>(config: ProgramConfig<Input, Output, Context>): Program<Input, Output, Context> {
  return defineProgram(config, "workflow");
}

/** One capture boundary for both public declarations. */
export function defineProgram<Input, Output, Context>(
  config: ProgramConfig<Input, Output, Context>,
  declarationKind: "agent" | "workflow",
  primaryModel?: ModelBinding<never, unknown>,
): Program<Input, Output, Context> {
  const programId = config.name ?? (config.run.name || (declarationKind === "agent" ? "Agent" : "Workflow"));
  const graph = captureProgram({
    programId,
    entrypoint: programId,
    contextSchema: config.context,
    declarationKind,
    primaryModel,
    declared: declaredSoFar(),
    source: authoredSource(),
  });
  return recordDeclaration(new ProgramHandle<Input, Output, Context>(programId, graph));
}

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

export class ProgramHandle<Input, Output, Context> implements ProgramBinding, Program<Input, Output, Context> {
  declare readonly [workflowBrand]: true;
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
objectFreeze(ProgramHandle.prototype);

export class ProgramInstance<Input, Output, Context> {
  constructor(
    readonly programRef: string,
    readonly artifactDigest: string,
  ) {}

  invoke(_input: Input): Promise<Output> {
    throw new Error("instance.invoke is called inside a compiled Agent body");
  }
}
