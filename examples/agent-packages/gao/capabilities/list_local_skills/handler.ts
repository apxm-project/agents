import { readdirSync } from "node:fs";
import { join } from "node:path";

import { tool } from "@apxm/frontend";

import { packageRoot } from "../handlers/context.js";

export const listLocalSkills = tool({
  name: "list_local_skills",
  description: "List Gao-owned local skills.",
})(() => {
  const skillsDir = join(packageRoot(), "skills");
  try {
    const rows = readdirSync(skillsDir, { withFileTypes: true })
      .filter((entry) => entry.isDirectory())
      .map((entry) => ({
        id: entry.name,
        path: `skills/${entry.name}/SKILL.md`,
      }))
      .sort((a, b) => a.id.localeCompare(b.id));
    return JSON.stringify(rows, null, 2);
  } catch {
    return "[]";
  }
});
