/** Structured task scope — parallel child composition with a typed join. */

import type { ProgramInstanceRef, ProgramInvokeSpec, ProgramRef } from "./program-instance.ts";
import { programInvokeOperands } from "./program-instance.ts";

type Json = Record<string, unknown>;

type GraphBuilderLike = {
  region(regionId: string, kind: string): unknown;
  programInvoke(nodeId: string, operands?: Json): unknown;
};

export type ProgramRecorder = {
  builder: GraphBuilderLike;
};

export class StructuredTaskScope {
  readonly program: ProgramRecorder;
  readonly regionId: string;
  readonly invokeNodeIds: string[] = [];

  constructor(program: ProgramRecorder, regionId: string) {
    this.program = program;
    this.regionId = regionId;
  }

  enter(): this {
    this.program.builder.region(this.regionId, "parallel_join");
    return this;
  }

  invokeProgram(
    nodeId: string,
    receiver: ProgramRef | ProgramInstanceRef,
    input?: Record<string, unknown>,
  ): this {
    const spec: ProgramInvokeSpec = { receiver, input };
    this.program.builder.programInvoke(nodeId, programInvokeOperands(spec) as Json);
    this.invokeNodeIds.push(nodeId);
    return this;
  }
}
