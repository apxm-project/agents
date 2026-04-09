export const ACTIVITIES = [
  { key: "dashboard", label: "Dashboard", shortcut: "1" },
  { key: "graph", label: "Explorer", shortcut: "2" },
  { key: "execution", label: "Execution", shortcut: "3" },
  { key: "reference", label: "Reference", shortcut: "4" },
  { key: "settings", label: "Settings", shortcut: "5" },
] as const;

export type ActivityKey = (typeof ACTIVITIES)[number]["key"];

// Keep backward compat alias
export type TabKey = ActivityKey;
export const TABS = ACTIVITIES;
