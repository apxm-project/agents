// The TypeScript twin of the Skilled reference Agent Program.
//
// It declares the same two Agent Skills the Python source does, in the same two
// forms, and loads both the same way. A Skill is not a Python construct: the
// marker, the two instruction sources, and `await skill.load()` are the one
// authoring surface every frontend projects.

import { Agent, Skill } from "@apxm/frontend";
import { source } from "@apxm/frontend/node";

source(import.meta.url);

type ReviewRequest = { change: string };
type ReviewGuidance = { instructions: string };
type SkilledProgram = ReturnType<typeof Agent<ReviewRequest, ReviewGuidance>>;

// Carried as a package file the folder contract recognizes and the integrity
// chain hashes.
const ReviewSkill = Skill("review", { entry: "skills/review/SKILL.md" });

// Written here, so editing it changes the artifact digest through the source
// bundle rather than through a package file digest.
const ToneSkill = Skill("tone", {
  text:
    "# Tone\n\n" +
    "Answer in the register the author wrote in. Prefer one concrete\n" +
    "sentence to three hedged ones.\n",
});

export const SkilledExample: SkilledProgram = Agent<ReviewRequest, ReviewGuidance>({
  name: "SkilledExample",
  async run() {
    await ToneSkill.load();
    const instructions = await ReviewSkill.load();
    return { instructions };
  },
});

export function buildSkilled(): SkilledProgram {
  return SkilledExample;
}
