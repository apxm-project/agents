// Shared typed semantics for Gao's package-local planning capabilities.
export type PermissionDecision = "allow" | "ask" | "deny";

export interface CapabilityCatalogEntry {
  id: string;
  description?: string;
  read_only?: boolean;
  permission?: PermissionDecision;
}

export interface PermissionPolicyEntry {
  capability_id: string;
  decision: PermissionDecision;
  reason?: string;
}

export interface PermissionPolicyInput {
  default_decision?: PermissionDecision;
  entries?: PermissionPolicyEntry[];
}

export interface ResolvedCapability {
  id: string;
  known: boolean;
  description?: string;
  read_only?: boolean;
  decision: PermissionDecision | "unknown";
  reason?: string;
}

const STOP_WORDS = new Set([
  "a",
  "an",
  "and",
  "as",
  "at",
  "be",
  "by",
  "for",
  "from",
  "in",
  "into",
  "is",
  "it",
  "of",
  "on",
  "or",
  "the",
  "then",
  "to",
  "with",
  "workflow",
]);

function normalizedWords(value: string): Set<string> {
  return new Set(
    value
      .toLowerCase()
      .replaceAll(/[^a-z0-9_-]+/g, " ")
      .split(/\s+/)
      .map((word) => word.trim())
      .filter((word) => word.length > 1 && !STOP_WORDS.has(word)),
  );
}

function escapeRegExp(value: string): string {
  return value.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
}

function capabilityScore(text: string, entry: CapabilityCatalogEntry): number {
  const normalizedText = text.toLowerCase();
  const idPattern = new RegExp(`(^|[^a-z0-9_-])${escapeRegExp(entry.id.toLowerCase())}([^a-z0-9_-]|$)`);
  if (idPattern.test(normalizedText)) {
    return 100;
  }

  const textWords = normalizedWords(text);
  const catalogWords = normalizedWords(`${entry.id} ${entry.description ?? ""}`);
  let score = 0;
  for (const word of catalogWords) {
    if (textWords.has(word)) {
      score += 1;
    }
  }
  return score;
}

export function resolveCapability(
  capabilityId: string,
  catalog: readonly CapabilityCatalogEntry[],
  policy: PermissionPolicyInput,
): ResolvedCapability {
  const catalogEntry = catalog.find((entry) => entry.id === capabilityId);
  const policyEntry = policy.entries?.find((entry) => entry.capability_id === capabilityId);
  const decision =
    policyEntry?.decision ?? catalogEntry?.permission ?? policy.default_decision ?? "unknown";

  return {
    id: capabilityId,
    known: catalogEntry != null,
    ...(catalogEntry?.description ? { description: catalogEntry.description } : {}),
    ...(typeof catalogEntry?.read_only === "boolean"
      ? { read_only: catalogEntry.read_only }
      : {}),
    decision,
    ...(policyEntry?.reason ? { reason: policyEntry.reason } : {}),
  };
}

export function capabilitiesMentionedIn(
  text: string,
  catalog: readonly CapabilityCatalogEntry[],
): CapabilityCatalogEntry[] {
  return catalog
    .map((entry) => ({ entry, score: capabilityScore(text, entry) }))
    .filter(({ score }) => score > 0)
    .sort((left, right) => right.score - left.score || left.entry.id.localeCompare(right.entry.id))
    .map(({ entry }) => entry);
}

export function splitWorkflowRequest(request: string): {
  triggers: string[];
  actions: string[];
} {
  const clauses = request
    .split(/(?:\r?\n|[.!?]+|\b(?:and then|then|after that)\b)/i)
    .map((clause) => clause.trim())
    .filter(Boolean);
  const triggers = clauses.filter((clause) => /^(?:after|at|every|on|when|whenever)\b/i.test(clause));
  const actions = clauses.filter((clause) => !triggers.includes(clause));
  return {
    triggers,
    actions: actions.length > 0 ? actions : [request.trim()],
  };
}
