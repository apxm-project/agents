// Defines typed generic program composition operands.

export type ProgramRef = {
  program_ref: string;
};

export type ProgramInstanceRef = {
  instance_node_id: string;
  program_ref: string;
};

export type ProgramNewSpec = {
  program_ref: string;
  initial_context?: Record<string, unknown>;
};

export type ProgramInvokeSpec = {
  receiver: ProgramRef | ProgramInstanceRef;
  input?: Record<string, unknown>;
};

/** Convert an instance reference to its canonical receiver operand. */
export function programInstanceReceiver(
  instance: ProgramInstanceRef,
): Record<string, string> {
  return { program_instance_ref: instance.instance_node_id };
}

/** Convert a typed creation request to canonical operands. */
export function programNewOperands(spec: ProgramNewSpec): Record<string, unknown> {
  const operands: Record<string, unknown> = { program_ref: spec.program_ref };
  if (spec.initial_context !== undefined) operands.initial_context = spec.initial_context;
  return operands;
}

/** Convert a typed invocation request to canonical operands. */
export function programInvokeOperands(spec: ProgramInvokeSpec): Record<string, unknown> {
  const receiver =
    "instance_node_id" in spec.receiver
      ? programInstanceReceiver(spec.receiver)
      : { program_ref: spec.receiver.program_ref };
  const operands: Record<string, unknown> = { receiver };
  if (spec.input !== undefined) operands.input = spec.input;
  return operands;
}
