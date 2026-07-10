import { tool } from "@apxm/frontend";

type EditArgs = {
  file_path?: unknown;
  before?: unknown;
  after?: unknown;
};

export const proposeEdit = tool({
  name: "edit",
  description: "Return a deterministic patch proposal without changing a file.",
})((raw: unknown) => {
  const args = (raw ?? {}) as EditArgs;
  const filePath = String(args.file_path ?? "").trim();
  const before = String(args.before ?? "");
  const after = String(args.after ?? "");
  if (!filePath || !before || before === after) {
    throw new Error("edit requires file_path, before, and a changed after value");
  }
  return JSON.stringify({
    file_path: filePath,
    before,
    after,
    mutates: false,
  });
});
