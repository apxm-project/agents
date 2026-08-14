// Defines Coder's read-only structured edit proposal Tool.
import { Tool } from "@apxm/agent-packaging";

/** Prepare a before/after proposal without mutating a file. */
export const proposeEdit = Tool.define({
  name: "edit",
  description: "Return a structured before/after proposal without changing a file.",
  // The handler states this about itself; nothing else in the package does.
  readOnly: true,
  input: Tool.object({
    additionalProperties: false,
    properties: {
      file_path: Tool.text({ required: true, minLength: 1 }),
      before: Tool.text({ required: true, minLength: 1 }),
      after: Tool.text({ required: true, minLength: 1 }),
    },
  }),
  run(args) {
    const file_path = args.file_path.trim();
    if (file_path.length === 0 || args.before === args.after) {
      throw new Error("edit requires a file path and a changed proposal");
    }
    return Tool.answer({ file_path, before: args.before, after: args.after, mutates: false });
  },
});
