import { useEffect } from "react";
import { useAppStore } from "@/store/app-store";
import { Overlay, CTX_TAB_SHORTCUTS } from "@/lib/constants";

export function useKeyboard() {
  const setOverlayView = useAppStore((s) => s.setOverlayView);
  const toggleInspector = useAppStore((s) => s.toggleInspector);
  const toggleContextPanel = useAppStore((s) => s.toggleContextPanel);
  const setNavigatorCollapsed = useAppStore((s) => s.setNavigatorCollapsed);
  const navigatorCollapsed = useAppStore((s) => s.navigatorCollapsed);
  const setContextPanelTab = useAppStore((s) => s.setContextPanelTab);
  const overlayView = useAppStore((s) => s.overlayView);

  useEffect(() => {
    function handleKeyDown(e: KeyboardEvent) {
      const tag = (e.target as HTMLElement).tagName;
      if (tag === "INPUT" || tag === "TEXTAREA" || tag === "SELECT") return;

      if (e.key === "Escape") {
        if (overlayView) {
          e.preventDefault();
          setOverlayView(null);
          return;
        }
      }

      if ((e.metaKey || e.ctrlKey) && !e.shiftKey) {
        if (e.key in CTX_TAB_SHORTCUTS) {
          e.preventDefault();
          setContextPanelTab(CTX_TAB_SHORTCUTS[e.key]!);
          return;
        }
        if (e.key === ".") {
          e.preventDefault();
          toggleContextPanel();
          return;
        }
        if (e.key === "b" || e.key === "B") {
          e.preventDefault();
          setNavigatorCollapsed(!navigatorCollapsed);
          return;
        }
      }

      if (e.key === "s" || e.key === "S") {
        if (!e.metaKey && !e.ctrlKey) {
          e.preventDefault();
          setOverlayView(overlayView === Overlay.STUDIO ? null : Overlay.STUDIO);
          return;
        }
      }

      if (e.key === "i" || e.key === "I") {
        e.preventDefault();
        toggleInspector();
        return;
      }
    }

    window.addEventListener("keydown", handleKeyDown);
    return () => window.removeEventListener("keydown", handleKeyDown);
  }, [setOverlayView, toggleInspector, toggleContextPanel, setNavigatorCollapsed, navigatorCollapsed, setContextPanelTab, overlayView]);
}
