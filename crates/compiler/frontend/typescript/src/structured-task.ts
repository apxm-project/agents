// Provides joined generic child invocation scopes.

import type {
  ProgramInstanceRef,
  ProgramInvokeSpec,
  ProgramRef,
} from "./program-instance.js";

export class StructuredTaskScope {
  readonly invokeNodeIds: string[] = [];
  private previousRegionId: string | undefined;

  constructor(
    readonly regionId: string,
    private readonly recordEntry: () => string | undefined,
    private readonly recordExit: (previous: string | undefined) => unknown,
    private readonly recordInvoke: (nodeId: string, spec: ProgramInvokeSpec) => unknown,
  ) {}

  /** Enter the attached parallel region. */
  enter(): this {
    this.previousRegionId = this.recordEntry();
    return this;
  }

  /** Leave the attached parallel region. */
  exit(): void {
    this.recordExit(this.previousRegionId);
  }

  /** Add an invocation that must complete before scope exit. */
  invokeProgram(
    nodeId: string,
    receiver: ProgramRef | ProgramInstanceRef,
    input?: Record<string, unknown>,
  ): this {
    this.recordInvoke(nodeId, { receiver, input });
    this.invokeNodeIds.push(nodeId);
    return this;
  }
}
