import { tool } from "@apxm/agent-packaging";

interface TestArgs {
  command: string;
}

/** Prepare a test command without executing it. */
export const prepareTest = tool({
  name: "test",
  description: "Prepare a single-line test command for review without executing it.",
  schema: {
    type: "object",
    properties: {
      command: { type: "string", minLength: 1, pattern: "^[^\\r\\n]+$" },
    },
    required: ["command"],
    additionalProperties: false,
  },
})((args: TestArgs) => {
  const command = args.command.trim();
  if (!command || command.includes("\n") || command.includes("\r")) {
    throw new Error("test requires a single-line command");
  }
  return { command, executes: false, mutates: false };
});
