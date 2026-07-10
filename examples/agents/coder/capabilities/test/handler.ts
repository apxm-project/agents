import { tool } from "@apxm/frontend";

type TestArgs = {
  command?: unknown;
};

export const prepareTest = tool({
  name: "test",
  description: "Return a deterministic test command without executing it.",
})((raw: unknown) => {
  const args = (raw ?? {}) as TestArgs;
  const command = String(args.command ?? "python -m unittest discover -s tests").trim();
  if (!command || command.includes("\n") || command.includes(";")) {
    throw new Error("test command must be a single workspace-confined command");
  }
  return JSON.stringify({ command, mutates: false });
});
