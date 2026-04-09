export const TABS = [
  { key: "graph", label: "Graph" },
  { key: "compiler", label: "Compiler" },
  { key: "session", label: "Session" },
  { key: "live", label: "Live" },
  { key: "ops", label: "Ops" },
  { key: "config", label: "Config" },
  { key: "replay", label: "Replay" },
] as const;

export type TabKey = (typeof TABS)[number]["key"];

export const PASS_META: Record<string, { label: string; description: string }> = {
  prompt_caching:    { label: "Prompt Caching",    description: "Cache repeated prompt prefixes" },
  memoization_hints: { label: "Memoization Hints", description: "Mark deterministic nodes for memoization" },
  pipeline_detection:{ label: "Pipeline Detection", description: "Detect linear chains for pipelining" },
};

export const AVAILABLE_PASSES = Object.keys(PASS_META);
