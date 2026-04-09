import { useEffect, useState } from "react";
import { useAppStore } from "@/store/app-store";
import { fetchSessions, fetchSession } from "@/api/session";
import { formatDuration, formatAbsoluteTimestamp, getErrorMessage, STATUS_COLORS, FALLBACK_STATUS_COLOR, statusBadgeStyle, stripNodePrefix } from "@/lib/format";

export function SessionView() {
  const sessions = useAppStore((s) => s.sessions);
  const setSessions = useAppStore((s) => s.setSessions);
  const sessionData = useAppStore((s) => s.sessionData);
  const setSessionData = useAppStore((s) => s.setSessionData);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    fetchSessions().then(setSessions).catch(() => {});
  }, [setSessions]);

  async function handleSelectSession(path: string) {
    setLoading(true);
    setError(null);
    try {
      const data = await fetchSession(path);
      setSessionData(data, path);
    } catch (e) {
      setError(getErrorMessage(e));
    }
    setLoading(false);
  }

  return (
    <div className="view-panel">
      <div className="view-panel__header">
        <h2>Sessions</h2>
      </div>
      {error ? <div className="error-banner">{error}</div> : null}
      <div className="session-layout">
        <div className="session-list">
          {sessions.length === 0 ? (
            <div className="view-panel__empty">No sessions found. Run a graph with --emit-session to create one.</div>
          ) : null}
          {sessions.map((s) => {
            const color = STATUS_COLORS[s.status] ?? FALLBACK_STATUS_COLOR;
            return (
              <button
                key={s.id}
                type="button"
                className="session-list__item"
                onClick={() => handleSelectSession(`~/.apxm/sessions/${s.id}`)}
              >
                <div className="session-list__name">{s.graph_name ?? s.id}</div>
                <div className="session-list__meta">
                  <span style={statusBadgeStyle(color)}>{s.status}</span>
                  <span>{formatAbsoluteTimestamp(s.started_at)}</span>
                </div>
              </button>
            );
          })}
        </div>
        <div className="session-detail">
          {loading ? <div className="viewer-status"><div className="viewer-status__spinner" /><p>Loading session...</p></div> : null}
          {sessionData && !loading ? (
            <div className="session-content">
              <div className="session-header">
                <h3>{sessionData.manifest?.graph_name ?? "Session"}</h3>
                <span style={statusBadgeStyle(STATUS_COLORS[sessionData.manifest?.status ?? ""] ?? FALLBACK_STATUS_COLOR)}>
                  {sessionData.manifest?.status ?? "unknown"}
                </span>
              </div>
              {sessionData.manifest?.duration_ms != null ? (
                <div className="session-metrics">Duration: {formatDuration(sessionData.manifest.duration_ms)}</div>
              ) : null}
              <div className="session-trace">
                <strong>Trace events: {sessionData.trace.length}</strong>
              </div>
              {sessionData.node_statuses ? (
                <div className="session-nodes">
                  <strong>Node Statuses:</strong>
                  <div className="node-status-grid">
                    {Object.entries(sessionData.node_statuses).map(([key, val]) => {
                      const status = (val as Record<string, unknown>)?.status as string ?? "unknown";
                      const color = STATUS_COLORS[status] ?? FALLBACK_STATUS_COLOR;
                      return (
                        <div key={key} className="node-status-cell" style={{ borderLeftColor: color }}>
                          <div className="node-status-cell__name">{stripNodePrefix(key)}</div>
                          <div className="node-status-cell__status" style={statusBadgeStyle(color)}>{status}</div>
                        </div>
                      );
                    })}
                  </div>
                </div>
              ) : null}
            </div>
          ) : null}
          {!sessionData && !loading ? (
            <div className="view-panel__empty">Select a session to view details.</div>
          ) : null}
        </div>
      </div>
    </div>
  );
}
