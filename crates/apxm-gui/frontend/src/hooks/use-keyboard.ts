import { useEffect } from "react";
import { useAppStore } from "@/store/app-store";
import type { TabKey } from "@/lib/constants";

const TAB_KEYS: Record<string, TabKey> = {
  "1": "graph",
  "2": "compiler",
  "3": "session",
  "4": "live",
  "5": "ops",
  "6": "config",
  "7": "replay",
};

export function useKeyboard() {
  const setActiveTab = useAppStore((s) => s.setActiveTab);
  const toggleInspector = useAppStore((s) => s.toggleInspector);

  useEffect(() => {
    function handleKeyDown(e: KeyboardEvent) {
      // Don't handle when typing in inputs
      const tag = (e.target as HTMLElement).tagName;
      if (tag === "INPUT" || tag === "TEXTAREA" || tag === "SELECT") return;

      if (e.key in TAB_KEYS) {
        e.preventDefault();
        setActiveTab(TAB_KEYS[e.key]!);
        return;
      }

      if (e.key === "i" || e.key === "I") {
        e.preventDefault();
        toggleInspector();
        return;
      }
    }

    window.addEventListener("keydown", handleKeyDown);
    return () => window.removeEventListener("keydown", handleKeyDown);
  }, [setActiveTab, toggleInspector]);
}
