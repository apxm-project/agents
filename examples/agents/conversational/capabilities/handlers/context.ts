import { readFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

export const SUMMARY_KEY = "conversation.summary";

const PROMPT_PATHS: Record<string, string> = {
  context: "prompts/context.md",
  persona: "prompts/persona.md",
};

let cachedRoot: string | null = null;

function agentRoot(): string {
  if (cachedRoot) {
    return cachedRoot;
  }
  const here = dirname(fileURLToPath(import.meta.url));
  cachedRoot = resolve(here, "..", "..");
  return cachedRoot;
}

function prompt(name: string): string {
  const relative = PROMPT_PATHS[name];
  if (!relative) {
    throw new Error(`unknown prompt: ${name}`);
  }
  const root = agentRoot();
  const path = resolve(root, relative);
  if (!path.startsWith(root)) {
    throw new Error(`path escapes agent root: ${relative}`);
  }
  return readFileSync(path, "utf8").trim();
}

export function context_prompt(summary: string, recent: string): string {
  return [
    prompt("context"),
    `Running summary:\n${summary || "(none)"}`,
    `Recent turns:\n${recent || "(none)"}`,
  ].join("\n\n");
}
