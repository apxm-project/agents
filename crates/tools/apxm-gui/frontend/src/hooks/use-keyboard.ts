import { useEffect } from "react";
import { useAppStore } from "@/store/app-store";
import type { ActivityKey } from "@/lib/constants";

const ACTIVITY_KEYS: Record<string, ActivityKey> = {
  "1": "dashboard",
  "2": "graph",
  "3": "execution",
  "4": "live",
  "5": "replay",
  "6": "reference",
  "7": "settings",
};

export function useKeyboard() {
  const setActiveTab = useAppStore((s) => s.setActiveTab);
  const toggleInspector = useAppStore((s) => s.toggleInspector);

  useEffect(() => {
    function handleKeyDown(e: KeyboardEvent) {
      const tag = (e.target as HTMLElement).tagName;
      if (tag === "INPUT" || tag === "TEXTAREA" || tag === "SELECT") return;

      if (e.key in ACTIVITY_KEYS) {
        e.preventDefault();
        setActiveTab(ACTIVITY_KEYS[e.key]!);
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
