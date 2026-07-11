// Lists package-local files without permitting path traversal.
import { readdirSync } from "node:fs";
import { isAbsolute, relative, resolve } from "node:path";

import { tool } from "@apxm/frontend";

import { packageRoot } from "../handlers/context.js";

interface ListFilesArgs {
  subpath?: string;
}

/** List files below the Gao package root. */
export const listFiles = tool({
  name: "list_files",
  description: "List files under a path relative to declared read roots.",
  schema: {
    type: "object",
    properties: { subpath: { type: "string" } },
    additionalProperties: false,
  },
})((args: ListFilesArgs) => {
  const root = packageRoot();
  const target = resolve(root, args.subpath ?? "");
  const targetRelative = relative(root, target);
  if (targetRelative.startsWith("..") || isAbsolute(targetRelative)) {
    throw new Error("subpath escapes Gao package");
  }
  try {
    const rows = readdirSync(target, { withFileTypes: true })
      .map((entry) => ({
        name: entry.name,
        path: relative(root, resolve(target, entry.name)),
        kind: entry.isDirectory() ? "dir" : "file",
      }))
      .sort((a, b) => a.path.localeCompare(b.path));
    return JSON.stringify(rows, null, 2);
  } catch {
    return "[]";
  }
});
