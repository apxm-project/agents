import { useAppStore } from "@/store/app-store";
import { formatPreciseTimestamp, STATUS_COLORS, FALLBACK_STATUS_COLOR, statusBadgeStyle } from "@/lib/format";
import { eventSummary } from "@/lib/event-utils";

export function LiveView() {
  const liveActive = useAppStore((s) => s.liveActive);
  const liveSessionPath = useAppStore((s) => s.liveSessionPath);
  const liveEvents = useAppStore((s) => s.liveEvents);
  const liveNodeStates = useAppStore((s) => s.liveNodeStates);
  const liveManifest = useAppStore((s) => s.liveManifest);
  const setLiveActive = useAppStore((s) => s.setLiveActive);
  const clearLive = useAppStore((s) => s.clearLive);

  function handleConnect() {
    const path = prompt("Enter session path:");
    if (path) setLiveActive(true, path);
  }

  const manifestStatus = (liveManifest?.status as string) ?? (liveActive ? "running" : "idle");
  const statusColor = STATUS_COLORS[manifestStatus] ?? FALLBACK_STATUS_COLOR;

  return (
    <div className="view-panel">
      <div className="view-panel__header">
        <h2>Live Execution</h2>
        <div className="view-panel__actions">
          {!liveActive ? (
            <button type="button" className="ghost-button" onClick={handleConnect}>Connect</button>
          ) : (
            <button type="button" className="ghost-button" onClick={clearLive}>Disconnect</button>
          )}
        </div>
      </div>
      <div className="live-status" style={statusBadgeStyle(statusColor)}>
        {liveActive ? `Connected: ${liveSessionPath}` : "Not connected"} — {manifestStatus}
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
          <div className="event-feed">
            {liveEvents.slice(-100).reverse().map((event, i) => (
              <div key={i} className="event-feed__item">
                <span className="event-feed__time">{formatPreciseTimestamp(event.timestamp)}</span>
                <span className="event-feed__kind">{event.kind}</span>
                <span className="event-feed__summary">{eventSummary(event)}</span>
              </div>
            ))}
          </div>
        </div>
      </div>
    </div>
  );
}
