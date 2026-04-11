// ── Overlay views (full-page overlays) ──────────────────────
export const Overlay = {
  STUDIO: "studio",
  REFERENCE: "reference",
  SETTINGS: "settings",
} as const;

export type OverlayKey = (typeof Overlay)[keyof typeof Overlay];
export type OverlayView = OverlayKey | null;

export const OVERLAY_LINKS = [
  { key: Overlay.STUDIO, label: "Studio", shortcut: "S" },
  { key: Overlay.REFERENCE, label: "Reference", shortcut: "R" },
  { key: Overlay.SETTINGS, label: "Settings", shortcut: "," },
] as const;

// ── Context panel tabs ──────────────────────────────────────
export const CtxTab = {
  GRAPH: "graph",
  SOURCE: "source",
  SESSIONS: "sessions",
  LIVE: "live",
} as const;

export type ContextTab = (typeof CtxTab)[keyof typeof CtxTab];

export const CONTEXT_TABS: { key: ContextTab; label: string }[] = [
  { key: CtxTab.GRAPH, label: "Graph" },
  { key: CtxTab.SOURCE, label: "Source" },
  { key: CtxTab.SESSIONS, label: "Sessions" },
  { key: CtxTab.LIVE, label: "Live" },
];

// ── View modes (Studio graph/source toggle) ─────────────────
export const ViewMode = {
  SOURCE: "source",
  GRAPH: "graph",
} as const;

export type ViewModeKey = (typeof ViewMode)[keyof typeof ViewMode];

// ── Keyboard shortcut → context tab mapping ─────────────────
export const CTX_TAB_SHORTCUTS: Record<string, ContextTab> = {
  "1": CtxTab.GRAPH,
  "2": CtxTab.SOURCE,
  "3": CtxTab.SESSIONS,
  "4": CtxTab.LIVE,
};

// ── Overlay labels (for header display) ─────────────────────
export const OVERLAY_LABELS: Record<OverlayKey, string> = {
  [Overlay.STUDIO]: "Studio",
  [Overlay.REFERENCE]: "Reference",
  [Overlay.SETTINGS]: "Settings",
};

// ── Chat roles ──────────────────────────────────────────────
export const Role = {
  USER: "user",
  ASSISTANT: "assistant",
  SYSTEM: "system",
} as const;

export type ChatRole = (typeof Role)[keyof typeof Role];

// ── SSE event types (agent stream) ──────────────────────────
export const SseEvent = {
  TOKEN: "token",
  TOOL_CALL: "tool_call",
  TOOL_RESULT: "tool_result",
  USAGE: "usage",
  DONE: "done",
  ERROR: "error",
} as const;

export type SseEventType = (typeof SseEvent)[keyof typeof SseEvent];

// ── Loading / error state keys ──────────────────────────────
export const LoadKey = {
  STARTUP: "startup",
  WORKFLOWS: "workflows",
  SOURCE: "source",
  GRAPH: "graph",
  COMPILE: "compile",
  SAVE: "save",
  ANALYSIS: "analysis",
} as const;
