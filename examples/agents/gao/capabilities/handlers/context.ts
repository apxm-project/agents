// Builds Gao's bounded, redacted turn context from package and host data.
import { readFileSync } from "node:fs";
import { dirname, isAbsolute, relative, resolve } from "node:path";
import { fileURLToPath } from "node:url";

import type { HookContext } from "@apxm/frontend";

export const SUMMARY_KEY = "gao:conversation:summary";

const MAX_SUPPLEMENT_TOKENS = 24_000;
const CHARS_PER_TOKEN_ESTIMATE = 4;
const MAX_CONFIG_CHARS = 500;

const SENSITIVE_KEY =
  /(?:api[_-]?key|token|secret|password|credential|auth)/i;

interface CapabilityInventoryEntry {
  capability: string;
  reason?: string;
}

interface CapabilityInventory {
  ready: CapabilityInventoryEntry[];
  needsConnect: CapabilityInventoryEntry[];
}

interface NodeKindEntry {
  kind: string;
  title: string;
  category: string;
}

const PROMPT_PATHS: Record<string, string> = {
  persona: "prompts/persona.md",
  workflow_authoring: "prompts/workflow-authoring.md",
  terminology: "prompts/apxm-terminology.md",
  safety: "prompts/safety.md",
  response_style: "prompts/response-style.md",
};

const PROMPT_ORDER = [
  "persona",
  "terminology",
  "workflow_authoring",
  "safety",
  "response_style",
] as const;

let cachedRoot: string | null = null;

export function packageRoot(): string {
  if (cachedRoot) {
    return cachedRoot;
  }
  const here = dirname(fileURLToPath(import.meta.url));
  const root = resolve(here, "..", "..");
  cachedRoot = root;
  return root;
}

function readText(relativePath: string): string {
  const root = packageRoot();
  const path = resolve(root, relativePath);
  const rel = relative(root, path);
  if (rel.startsWith("..") || isAbsolute(rel)) {
    throw new Error(`path escapes Gao package: ${relativePath}`);
  }
  return readFileSync(path, "utf8").trim();
}

export function prompt(name: string): string {
  const relative = PROMPT_PATHS[name];
  if (!relative) {
    throw new Error(`unknown prompt: ${name}`);
  }
  return readText(relative);
}

function redactValue(key: string, value: unknown): unknown {
  if (SENSITIVE_KEY.test(key)) {
    return "<redacted>";
  }
  if (typeof value === "string" && value.length > MAX_CONFIG_CHARS) {
    return `${value.slice(0, MAX_CONFIG_CHARS)}…`;
  }
  if (value && typeof value === "object" && !Array.isArray(value)) {
    return redactConfig(value as Record<string, unknown>);
  }
  if (Array.isArray(value)) {
    return value.map((item, index) => redactValue(String(index), item));
  }
  return value;
}

function redactConfig(config: Record<string, unknown>): Record<string, unknown> {
  return Object.fromEntries(
    Object.entries(config ?? {}).map(([key, value]) => [key, redactValue(key, value)]),
  );
}

function redactSnapshot(snapshot: Record<string, unknown>): Record<string, unknown> {
  const out = JSON.parse(JSON.stringify(snapshot ?? {})) as Record<string, unknown>;
  const canvas = out.canvas;
  if (canvas && typeof canvas === "object" && !Array.isArray(canvas)) {
    const nodes = (canvas as Record<string, unknown>).nodes;
    if (Array.isArray(nodes)) {
      for (const node of nodes) {
        if (node && typeof node === "object" && !Array.isArray(node)) {
          const config = (node as Record<string, unknown>).config;
          if (config && typeof config === "object" && !Array.isArray(config)) {
            (node as Record<string, unknown>).config = redactConfig(
              config as Record<string, unknown>,
            );
          }
        }
      }
    }
  }
  return out;
}

function parseNodeKindEntry(value: unknown): NodeKindEntry | null {
  if (!value || typeof value !== "object" || Array.isArray(value)) {
    return null;
  }
  const record = value as Record<string, unknown>;
  if (typeof record.kind !== "string" || record.kind.trim() === "") {
    return null;
  }
  return {
    kind: record.kind,
    title: typeof record.title === "string" ? record.title : record.kind,
    category: typeof record.category === "string" ? record.category : "other",
  };
}

function parseNodeKindCatalog(value: unknown): NodeKindEntry[] {
  return Array.isArray(value)
    ? value.map(parseNodeKindEntry).filter((entry): entry is NodeKindEntry => entry != null)
    : [];
}

export function renderNodeKindCatalog(value: unknown): string {
  const kinds = parseNodeKindCatalog(value);
  if (kinds.length === 0) {
    return (
      "(host did not supply `nodeKinds`; do not emit Apply workflow JSON. " +
      "High-level planning and clarification are still allowed.)"
    );
  }
  const byCategory = new Map<string, string[]>();
  for (const entry of kinds) {
    const category = String(entry.category ?? "other");
    const line = `- \`${entry.kind}\` — ${entry.title ?? entry.kind}`;
    const bucket = byCategory.get(category) ?? [];
    bucket.push(line);
    byCategory.set(category, bucket);
  }
  const lines: string[] = [];
  for (const category of [...byCategory.keys()].sort()) {
    lines.push(`**${category}**`);
    lines.push(...(byCategory.get(category) ?? []));
  }
  return lines.join("\n");
}

function parseInventoryEntry(value: unknown): CapabilityInventoryEntry | null {
  if (!value || typeof value !== "object" || Array.isArray(value)) {
    return null;
  }
  const record = value as Record<string, unknown>;
  if (typeof record.capability !== "string" || record.capability.trim() === "") {
    return null;
  }
  return {
    capability: record.capability,
    ...(typeof record.reason === "string" ? { reason: record.reason } : {}),
  };
}

function parseInventoryEntries(value: unknown): CapabilityInventoryEntry[] {
  return Array.isArray(value)
    ? value.map(parseInventoryEntry).filter((entry): entry is CapabilityInventoryEntry => entry != null)
    : [];
}

function parseCapabilityInventory(value: unknown): CapabilityInventory | null {
  if (!value || typeof value !== "object" || Array.isArray(value)) {
    return null;
  }
  const record = value as Record<string, unknown>;
  return {
    ready: parseInventoryEntries(record.ready),
    needsConnect: parseInventoryEntries(record.needsConnect),
  };
}

export function renderCapabilityInventorySection(
  inventory: CapabilityInventory | null,
): string {
  if (!inventory) {
    return (
      "- Ready now: (none)\n" +
      "- Status: host did not supply `capabilityInventory`; do not emit workflow tool nodes " +
      "or claim provider availability. Non-tool planning and clarification are still allowed."
    );
  }
  const { ready, needsConnect } = inventory;
  const readyLine =
    ready.map((entry) => String(entry.capability ?? "")).filter(Boolean).join(", ") || "(none)";
  const lines = [`- Ready now: ${readyLine}`];
  if (needsConnect.length > 0) {
    const needs = needsConnect
      .map(
        (entry) =>
          `${entry.capability ?? "unknown"} (${entry.reason ?? "not connected"})`,
      )
      .join(", ");
    lines.push(`- Needs connection: ${needs}`);
  }
  return lines.join("\n");
}

function renderPackagePrompts(): string {
  const parts: string[] = [];
  for (const name of PROMPT_ORDER) {
    try {
      parts.push(prompt(name));
    } catch {
      continue;
    }
  }
  return parts.join("\n\n");
}

function renderPageSection(snapshot: Record<string, unknown>): string {
  const page = (snapshot.page as Record<string, unknown> | undefined) ?? {};
  const lines = [
    `- View: ${page.view ?? "(unknown)"}`,
    `- Chat surface: ${page.chatSurface ?? "(unknown)"}`,
  ];
  const tab = page.activeTab as Record<string, unknown> | undefined;
  if (tab) {
    lines.push(`- Active tab: ${tab.name ?? "(unnamed)"} (${tab.id ?? "?"})`);
  }
  return lines.join("\n");
}

function renderWorkflowSection(snapshot: Record<string, unknown>): string {
  const workflow = (snapshot.workflow as Record<string, unknown> | undefined) ?? {};
  const lines = [`- Mode: ${workflow.mode ?? "(unknown)"}`];
  if (workflow.runError) {
    lines.push(`- Run error: ${workflow.runError}`);
  }
  const grants = workflow.capabilityGrants;
  if (Array.isArray(grants) && grants.length > 0) {
    lines.push(`- Workflow capability grants: ${grants.join(", ")}`);
  }
  return lines.join("\n");
}

function renderCanvasSection(snapshot: Record<string, unknown>): string {
  const canvas = (snapshot.canvas as Record<string, unknown> | undefined) ?? {};
  const lines = [
    `- Graph: ${canvas.nodeCount ?? 0} node(s), ${canvas.edgeCount ?? 0} edge(s)`,
  ];
  if (canvas.selectedNodeId) {
    lines.push(`- Selected node id: ${canvas.selectedNodeId}`);
  }
  const nodes = canvas.nodes;
  if (Array.isArray(nodes) && nodes.length > 0) {
    lines.push(`- Nodes JSON:\n\`\`\`json\n${JSON.stringify(nodes, null, 2)}\n\`\`\``);
  }
  return lines.join("\n");
}

function renderRunEvidenceSection(snapshot: Record<string, unknown>): string | null {
  const runEvidence = snapshot.runEvidence;
  if (!Array.isArray(runEvidence) || runEvidence.length === 0) {
    return null;
  }
  return `\`\`\`json\n${JSON.stringify(runEvidence, null, 2)}\n\`\`\``;
}

function renderOpenDocsSection(snapshot: Record<string, unknown>): string | null {
  const page = (snapshot.page as Record<string, unknown> | undefined) ?? {};
  const openTabs = page.openTabs;
  if (!Array.isArray(openTabs) || openTabs.length <= 1) {
    return null;
  }
  const labels = openTabs.map((tab) => {
    const record = tab as Record<string, unknown>;
    const active = record.active ? " (active)" : "";
    return `${record.name ?? "tab"}${active}`;
  });
  return `- Open tabs: ${labels.join(", ")}`;
}

async function tokenCount(text: string, ctx: HookContext): Promise<number> {
  try {
    return Math.max(1, await ctx.countTokens(text));
  } catch {
    return Math.max(1, Math.floor(text.length / CHARS_PER_TOKEN_ESTIMATE));
  }
}

function renderSections(sections: Array<[string, string | null | undefined]>): string {
  const header = "Studio operator context (read-only snapshot for this turn):";
  const body = sections
    .filter(([, text]) => Boolean(text))
    .map(([title, text]) => `## ${title}\n${text}`)
    .join("\n\n");
  return body ? `${header}\n\n${body}` : header;
}

export async function renderStudioContextSupplement(
  ctx: HookContext,
  snapshotInput: unknown,
): Promise<string> {
  const snapshot = redactSnapshot(
    snapshotInput && typeof snapshotInput === "object" && !Array.isArray(snapshotInput)
      ? (snapshotInput as Record<string, unknown>)
      : {},
  );
  const inventory = parseCapabilityInventory(snapshot.capabilityInventory);

  const coreSections: Array<[string, string]> = [
    ["Package prompts", renderPackagePrompts()],
    ["Page", renderPageSection(snapshot)],
    ["Workflow", renderWorkflowSection(snapshot)],
    ["Canvas", renderCanvasSection(snapshot)],
    ["Studio node kinds", renderNodeKindCatalog(snapshot.nodeKinds)],
    ["Available capabilities", renderCapabilityInventorySection(inventory)],
  ];

  let optionalSections: Array<[string, string | null]> = [
    ["Run evidence", renderRunEvidenceSection(snapshot)],
    ["Open tabs", renderOpenDocsSection(snapshot)],
  ].filter((entry): entry is [string, string] => entry[1] != null);

  let text = renderSections([...coreSections, ...optionalSections]);

  while ((await tokenCount(text, ctx)) > MAX_SUPPLEMENT_TOKENS && optionalSections.length > 0) {
    optionalSections = optionalSections.slice(1);
    text = renderSections([...coreSections, ...optionalSections]);
  }

  if ((await tokenCount(text, ctx)) > MAX_SUPPLEMENT_TOKENS) {
    const budgetChars = MAX_SUPPLEMENT_TOKENS * CHARS_PER_TOKEN_ESTIMATE;
    if (text.length > budgetChars) {
      text =
        text.slice(0, budgetChars) +
        `\n\n[Context truncated at the ${MAX_SUPPLEMENT_TOKENS}-token budget.]`;
    }
  }

  return text;
}
