import { readdirSync } from "node:fs";
import { join, relative } from "node:path";

import { tool } from "@apxm/frontend";

import { packageRoot } from "../handlers/context.js";

export const listFiles = tool({
  name: "list_files",
  description: "List files under a path relative to declared read roots.",
})((subpath: unknown) => {
  const root = packageRoot();
  const target = join(root, String(subpath ?? ""));
  try {
    const rows = readdirSync(target, { withFileTypes: true })
      .map((entry) => ({
        name: entry.name,
        path: relative(root, join(target, entry.name)),
        kind: entry.isDirectory() ? "dir" : "file",
      }))
      .sort((a, b) => a.path.localeCompare(b.path));
    return JSON.stringify(rows, null, 2);
  } catch {
    return "[]";
  }
});
