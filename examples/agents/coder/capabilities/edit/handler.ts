import { tool } from "@apxm/frontend";

interface EditArgs {
  file_path: string;
  before: string;
  after: string;
}

/** Prepare a before/after proposal without mutating the file. */
export const proposeEdit = tool({
  name: "edit",
  description: "Return a structured before/after proposal without changing a file.",
  schema: {
    type: "object",
    properties: {
      file_path: { type: "string", minLength: 1 },
      before: { type: "string", minLength: 1 },
      after: { type: "string", minLength: 1 },
    },
    required: ["file_path", "before", "after"],
    additionalProperties: false,
  },
})((args: EditArgs) => {
  const filePath = args.file_path.trim();
  const before = args.before;
  const after = args.after;
  if (!filePath || !before || before === after) {
    throw new Error("edit requires file_path, before, and a changed after value");
  }
  return {
    file_path: filePath,
    before,
    after,
    mutates: false,
  };
});
