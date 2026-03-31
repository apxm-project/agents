/**
 * Shared utilities for workflow-test flows.
 */
import { readFileSync, existsSync } from "fs";
import { join } from "path";

const AGENTS_DIR = ".agents";
const SKILLS_DIR = "skills";
const SKILL_FILENAME = "SKILL.md";

/**
 * Read a skill's SKILL.md content from the project's .agents/ directory.
 * Returns the full file content (frontmatter + body).
 */
export function readSkill(projectRoot: string, skillName: string): string {
  const skillPath = join(projectRoot, AGENTS_DIR, SKILLS_DIR, skillName, SKILL_FILENAME);
  if (!existsSync(skillPath)) {
    return `[Skill '${skillName}' not found at ${skillPath}]`;
  }
  return readFileSync(skillPath, "utf-8");
}

/**
 * Build a prompt section embedding one or more skills.
 */
export function embedSkills(projectRoot: string, skillNames: string[]): string {
  const sections = skillNames
    .map((name) => {
      const content = readSkill(projectRoot, name);
      return `### Skill: ${name}\n\n${content}`;
    })
    .join("\n\n");

  return `## Available Skills\n\n${sections}`;
}

/**
 * Run a shell command and return execution config for an action node.
 */
export function shellExec(
  command: string,
  args: string[] = [],
  cwd?: string
): { command: string; args: string[]; cwd?: string; shell: boolean } {
  return { command, args, cwd, shell: true };
}
