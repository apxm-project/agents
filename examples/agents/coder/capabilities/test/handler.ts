// Defines Coder's read-only test-command proposal Tool.
import { Tool } from "@apxm/agent-packaging";

interface TestArgs {
  command: string;
}

/** Prepare a test command without executing it. */
export const prepareTest = Tool.define({
  name: "test",
  description: "Prepare a single-line test command for review without executing it.",
  input: Tool.object<TestArgs>({
    command: Tool.text({ minLength: 1 }),
  }),
  run(args) {
    const command = args.command.trim();
    if (command.length === 0 || command.includes("\n") || command.includes("\r")) {
      throw new Error("test requires a single-line command");
    }
    return Tool.answer({ command, executes: false, mutates: false });
  },
});
