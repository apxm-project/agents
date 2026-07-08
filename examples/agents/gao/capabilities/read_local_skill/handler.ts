import { readFileSync } from "node:fs";
import { join, resolve } from "node:path";

import { tool } from "@apxm/frontend";

import { packageRoot } from "../handlers/context.js";

export const readLocalSkill = tool({
  name: "read_local_skill",
  description: "Read a Gao-owned skill document by skill id.",
})((skillId: unknown) => {
  const id = String(skillId ?? "").trim();
  if (!id) {
    throw new Error("skill_id is required");
  }
  const root = packageRoot();
  const path = resolve(root, "skills", id, "SKILL.md");
  if (!path.startsWith(resolve(root, "skills"))) {
    throw new Error("skill path escapes Gao package");
  }
  return readFileSync(path, "utf8");
});
