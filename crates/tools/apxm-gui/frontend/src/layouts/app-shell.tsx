import { useAppStore } from "@/store/app-store";
import { NavigatorPanel } from "@/components/navigator-panel";
import { ContextPanel } from "@/components/context-panel";
import { ChatView } from "@/views/chat-view";
import { GraphView } from "@/views/graph-view";
import { ReferenceView } from "@/views/reference-view";
import { ConfigView } from "@/views/config-view";
import { Overlay, OVERLAY_LABELS, LoadKey } from "@/lib/constants";

export function AppShell() {
  const overlayView = useAppStore((s) => s.overlayView);
  const setOverlayView = useAppStore((s) => s.setOverlayView);
  const contextPanelOpen = useAppStore((s) => s.contextPanelOpen);
  const navigatorCollapsed = useAppStore((s) => s.navigatorCollapsed);
  const setNavigatorCollapsed = useAppStore((s) => s.setNavigatorCollapsed);
  const toggleContextPanel = useAppStore((s) => s.toggleContextPanel);
  const errors = useAppStore((s) => s.errors);

  // Full-page overlay views (Studio, Reference, Settings)
  if (overlayView) {
    return (
      <div className="app-shell app-shell--overlay">
        <div className="overlay-header">
          <button
            type="button"
            className="overlay-back-btn"
            onClick={() => setOverlayView(null)}
          >
            <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2">
              <path d="M19 12H5M12 19l-7-7 7-7" />
            </svg>
            Back
          </button>
          <span className="overlay-title">
            {OVERLAY_LABELS[overlayView]}
          </span>
        </div>
        <div className="overlay-content">
          {overlayView === Overlay.STUDIO && <GraphView />}
          {overlayView === Overlay.REFERENCE && <ReferenceView />}
          {overlayView === Overlay.SETTINGS && <ConfigView />}
        </div>
      </div>
    );
  }

  // Default three-column layout
  return (
    <div className="app-shell app-shell--three-col">
      {/* Error banners */}
      {(errors[LoadKey.STARTUP] || errors[LoadKey.GRAPH]) && (
        <div className="app-errors">
          {errors[LoadKey.STARTUP] && <div className="error-banner">{errors[LoadKey.STARTUP]}</div>}
          {errors[LoadKey.GRAPH] && <div className="error-banner">{errors[LoadKey.GRAPH]}</div>}
        </div>
      )}

      {/* Navigator (left) */}
      {!navigatorCollapsed && (
        <aside className="app-navigator">
          <NavigatorPanel />
        </aside>
      )}

      {/* Chat (center) */}
      <main className="app-chat">
        {/* Navigator collapse/expand toggle */}
        <button
          type="button"
          className="panel-toggle panel-toggle--left"
          onClick={() => setNavigatorCollapsed(!navigatorCollapsed)}
          title={navigatorCollapsed ? "Show navigator" : "Hide navigator"}
        >
          <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2">
            {navigatorCollapsed
              ? <path d="M9 18l6-6-6-6" />
              : <path d="M15 18l-6-6 6-6" />
            }
          </svg>
        </button>

        <ChatView />

        {/* Context panel toggle (when closed) */}
        {!contextPanelOpen && (
          <button
            type="button"
            className="panel-toggle panel-toggle--right"
            onClick={toggleContextPanel}
            title="Show context panel"
          >
            <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2">
              <path d="M15 18l-6-6 6-6" />
            </svg>
          </button>
        )}
      </main>

      {/* Context Panel (right) */}
      {contextPanelOpen && (
        <aside className="app-context">
          <ContextPanel />
        </aside>
      )}
    </div>
  );
}
