// Defines Coder's read-only test-command proposal Tool.
import { Tool } from "@apxm/agent-packaging";

/** Prepare a test command without executing it. */
export const prepareTest = Tool.define({
  name: "test",
  description: "Prepare a single-line test command for review without executing it.",
  // The handler states this about itself; nothing else in the package does.
  readOnly: true,
  input: Tool.object({
    additionalProperties: false,
    properties: {
      command: Tool.text({ required: true, minLength: 1 }),
    },
  }),
  run(args) {
    const command = args.command.trim();
    if (command.length === 0 || command.includes("\n") || command.includes("\r")) {
      throw new Error("test requires a single-line command");
    }
    return Tool.answer({ command, executes: false, mutates: false });
  },
});
