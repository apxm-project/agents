// Reads one Gao-local skill document by validated skill id.
import { readFileSync } from "node:fs";
import { isAbsolute, relative, resolve } from "node:path";

import { tool } from "@apxm/frontend";

import { packageRoot } from "../handlers/context.js";

interface ReadLocalSkillArgs {
  skill_id: string;
}

/** Read a Gao-local skill document. */
export const readLocalSkill = tool({
  name: "read_local_skill",
  description: "Read a Gao-owned skill document by skill id.",
  schema: {
    type: "object",
    properties: { skill_id: { type: "string", pattern: "^[a-z0-9][a-z0-9-]*$" } },
    required: ["skill_id"],
    additionalProperties: false,
  },
})((args: ReadLocalSkillArgs) => {
  const id = args.skill_id.trim();
  if (!/^[a-z0-9][a-z0-9-]*$/.test(id)) {
    throw new Error("skill_id must be a lowercase package id");
  }
  const root = packageRoot();
  const path = resolve(root, "skills", id, "SKILL.md");
  const skillsRoot = resolve(root, "skills");
  const skillRelative = relative(skillsRoot, path);
  if (skillRelative.startsWith("..") || isAbsolute(skillRelative)) {
    throw new Error("skill path escapes Gao package");
  }
  return readFileSync(path, "utf8");
});
