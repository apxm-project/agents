// Defines Coder's read-only structured edit proposal Tool.
import { Tool } from "@apxm/agent-packaging";

interface EditArgs {
  file_path: string;
  before: string;
  after: string;
}

/** Prepare a before/after proposal without mutating a file. */
export const proposeEdit = Tool.define({
  name: "edit",
  description: "Return a structured before/after proposal without changing a file.",
  input: Tool.object<EditArgs>({
    file_path: Tool.text({ minLength: 1 }),
    before: Tool.text({ minLength: 1 }),
    after: Tool.text({ minLength: 1 }),
  }),
  run(args) {
    const file_path = args.file_path.trim();
    if (file_path.length === 0 || args.before === args.after) {
      throw new Error("edit requires a file path and a changed proposal");
    }
    return Tool.answer({ file_path, before: args.before, after: args.after, mutates: false });
  },
});
