import { useEffect } from "react";
import { useAppStore } from "@/store/app-store";
import { fetchSessions } from "@/api/session";
import { STATUS_COLORS, FALLBACK_STATUS_COLOR, statusBadgeStyle } from "@/lib/format";
import { SessionPicker } from "@/components/shared/session-picker";
import { EventFeed } from "@/components/shared/event-feed";

export function LiveView() {
  const liveActive = useAppStore((s) => s.liveActive);
  const liveSessionPath = useAppStore((s) => s.liveSessionPath);
  const liveEvents = useAppStore((s) => s.liveEvents);
  const liveNodeStates = useAppStore((s) => s.liveNodeStates);
  const liveManifest = useAppStore((s) => s.liveManifest);
  const sessions = useAppStore((s) => s.sessions);
  const setSessions = useAppStore((s) => s.setSessions);
  const navigateToLive = useAppStore((s) => s.navigateToLive);
  const clearLive = useAppStore((s) => s.clearLive);

  useEffect(() => {
    if (!liveActive && sessions.length === 0) {
      fetchSessions().then(setSessions).catch(() => {});
    }
  }, [liveActive, sessions.length, setSessions]);

  const manifestStatus = (liveManifest?.status as string) ?? (liveActive ? "running" : "idle");
  const statusColor = STATUS_COLORS[manifestStatus] ?? FALLBACK_STATUS_COLOR;

  const runningSessions = sessions.filter((s) => s.status === "running");

  if (!liveActive) {
    return (
      <div className="view-panel">
        <div className="view-panel__header">
          <h2>Live Execution</h2>
        </div>
        <SessionPicker
          sessions={runningSessions}
          title="Running Sessions"
          emptyMessage="No running sessions. Execute a workflow with --emit-session to watch it live."
          actionLabel="Watch Live"
          actionVariant="live"
          onSelect={navigateToLive}
        />
      </div>
    );
  }

  return (
    <div className="view-panel">
      <div className="view-panel__header">
        <h2>Live Execution</h2>
        <div className="view-panel__actions">
          <button type="button" className="ghost-button" onClick={clearLive}>Disconnect</button>
        </div>
      </div>
      <div className="live-status" style={statusBadgeStyle(statusColor)}>
        Connected: {liveSessionPath} — {manifestStatus}
      </div>
      <div className="live-layout">
        <div className="live-nodes">
          <strong>Node States ({Object.keys(liveNodeStates).length})</strong>
          <div className="node-status-grid">
            {Object.entries(liveNodeStates).map(([id, status]) => {
              const color = STATUS_COLORS[status] ?? FALLBACK_STATUS_COLOR;
              return (
                <div key={id} className="node-status-cell" style={{ borderLeftColor: color }}>
                  <div className="node-status-cell__name">Node #{id}</div>
                  <div className="node-status-cell__status" style={statusBadgeStyle(color)}>{status}</div>
                </div>
              );
            })}
          </div>
        </div>
        <div className="live-events">
          <strong>Events ({liveEvents.length})</strong>
          <EventFeed events={liveEvents} />
        </div>
      </div>
    </div>
  );
}
