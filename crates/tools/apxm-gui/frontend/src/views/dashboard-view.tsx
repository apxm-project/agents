import { useEffect, useRef, useState } from "react";
import { useAppStore } from "@/store/app-store";
import { fetchWorkflows } from "@/api/workflows";
import { fetchHealth } from "@/api/health";
import { fetchSession, fetchSessions } from "@/api/session";
import { formatDuration, formatAbsoluteTimestamp, STATUS_COLORS, FALLBACK_STATUS_COLOR, statusBadgeStyle, sessionPath } from "@/lib/format";
import type { ActivityKey } from "@/lib/constants";

export function DashboardView() {
  const workflows = useAppStore((s) => s.workflows);
  const sessions = useAppStore((s) => s.sessions);
  const health = useAppStore((s) => s.health);
  const liveActive = useAppStore((s) => s.liveActive);
  const setWorkflows = useAppStore((s) => s.setWorkflows);
  const setSessions = useAppStore((s) => s.setSessions);
  const setHealth = useAppStore((s) => s.setHealth);
  const setActiveTab = useAppStore((s) => s.setActiveTab);
  const setGraphData = useAppStore((s) => s.setGraphData);
  const setSessionData = useAppStore((s) => s.setSessionData);
  const navigateToLive = useAppStore((s) => s.navigateToLive);
  const navigateToReplay = useAppStore((s) => s.navigateToReplay);

  const [lastUpdated, setLastUpdated] = useState<Date | null>(null);
  const [probing, setProbing] = useState(false);
  const intervalRef = useRef<ReturnType<typeof setInterval> | null>(null);

  useEffect(() => {
    fetchWorkflows().then(setWorkflows).catch(() => {});
    fetchSessions().then((s) => { setSessions(s); setLastUpdated(new Date()); }).catch(() => {});
    fetchHealth().then(setHealth).catch(() => {});
  }, [setWorkflows, setSessions, setHealth]);

  // Poll sessions every 15s
  useEffect(() => {
    intervalRef.current = setInterval(() => {
      fetchSessions().then((s) => { setSessions(s); setLastUpdated(new Date()); }).catch(() => {});
    }, 15000);
    return () => { if (intervalRef.current) clearInterval(intervalRef.current); };
  }, [setSessions]);

  function handleWorkflowClick(path: string) {
    import("@/api/graph").then(({ fetchGraph }) => {
      fetchGraph(path).then((graph) => {
        setGraphData(graph, path);
        setActiveTab("graph" as ActivityKey);
      }).catch(() => {});
    });
  }

  function handleSessionClick(sessionId: string) {
    const path = sessionPath(sessionId);
    fetchSession(path).then((data) => {
      setSessionData(data, path);
      setActiveTab("execution" as ActivityKey);
    }).catch(() => {
      setActiveTab("execution" as ActivityKey);
    });
  }

  async function handleProbeHealth() {
    setProbing(true);
    try {
      const h = await fetchHealth({ probe: true });
      setHealth(h);
    } catch {}
    setProbing(false);
  }

  const recentSessions = sessions.slice(0, 10);
  const runningSessions = sessions.filter((s) => s.status === "running");

  return (
    <div className="view-panel">
      <div className="view-panel__header">
        <h2>Dashboard</h2>
        {lastUpdated ? (
          <span className="view-panel__updated">
            Updated {lastUpdated.toLocaleTimeString()}
          </span>
        ) : null}
      </div>

      {liveActive ? (
        <div className="live-indicator" onClick={() => setActiveTab("live" as ActivityKey)}>
          <span className="live-indicator__dot" />
          <span>Live session active</span>
        </div>
      ) : null}

      {runningSessions.length > 0 && !liveActive ? (
        <div className="live-indicator live-indicator--subtle" onClick={() => setActiveTab("live" as ActivityKey)}>
          <span className="live-indicator__dot" />
          <span>{runningSessions.length} running session{runningSessions.length !== 1 ? "s" : ""}</span>
        </div>
      ) : null}

      <div className="dashboard-grid">
        {/* Workflows column */}
        <div className="dashboard-card">
          <div className="dashboard-card__header">
            <h3>Workflows</h3>
            <span className="dashboard-card__count">{workflows.length}</span>
          </div>
          <div className="dashboard-card__body">
            {workflows.length === 0 ? (
              <div className="dashboard-empty">No workflows found in project directory.</div>
            ) : null}
            {workflows.map((w) => (
              <button
                key={w.path}
                type="button"
                className="dashboard-workflow"
                onClick={() => handleWorkflowClick(w.path)}
                title={w.relative_path}
              >
                <div className="dashboard-workflow__name">{w.name}</div>
                <div className="dashboard-workflow__meta">
                  {w.node_count != null ? (
                    <span className="dashboard-workflow__nodes">{w.node_count} nodes</span>
                  ) : null}
                  {w.category ? <span className="dashboard-workflow__category">{w.category}</span> : null}
                </div>
              </button>
            ))}
          </div>
        </div>

        {/* Recent Sessions column */}
        <div className="dashboard-card">
          <div className="dashboard-card__header">
            <h3>Recent Runs</h3>
            <span className="dashboard-card__count">{sessions.length}</span>
          </div>
          <div className="dashboard-card__body">
            {recentSessions.length === 0 ? (
              <div className="dashboard-empty">No sessions yet. Execute a workflow to create one.</div>
            ) : null}
            {recentSessions.map((s) => {
              const color = STATUS_COLORS[s.status] ?? FALLBACK_STATUS_COLOR;
              return (
                <div key={s.id} className="dashboard-session">
                  <button
                    type="button"
                    className="dashboard-session__main"
                    onClick={() => handleSessionClick(s.id)}
                  >
                    <div className="dashboard-session__name">{s.graph_name ?? s.id}</div>
                    <div className="dashboard-session__meta">
                      <span style={statusBadgeStyle(color)}>{s.status}</span>
                      <span>{formatAbsoluteTimestamp(s.started_at)}</span>
                      {s.duration_ms != null ? (
                        <span>{formatDuration(s.duration_ms)}</span>
                      ) : null}
                    </div>
                  </button>
                  <div className="dashboard-session__actions">
                    {s.status === "running" ? (
                      <button
                        type="button"
                        className="session-action-btn session-action-btn--live"
                        onClick={() => navigateToLive(sessionPath(s.id))}
                      >
                        Watch Live
                      </button>
                    ) : null}
                    {s.status === "completed" || s.status === "failed" ? (
                      <button
                        type="button"
                        className="session-action-btn"
                        onClick={() => navigateToReplay(sessionPath(s.id))}
                      >
                        Replay
                      </button>
                    ) : null}
                  </div>
                </div>
              );
            })}
          </div>
        </div>

        {/* Health column */}
        <div className="dashboard-card">
          <div className="dashboard-card__header">
            <h3>Infrastructure</h3>
            <button type="button" className="ghost-button ghost-button--sm" onClick={handleProbeHealth} disabled={probing}>
              {probing ? "Probing..." : "Refresh Health"}
            </button>
          </div>
          <div className="dashboard-card__body">
            {health == null ? (
              <div className="dashboard-empty">Loading health status...</div>
            ) : (
              <>
                <div className="dashboard-health-section">
                  <strong>Backends ({health.backends.length})</strong>
                  {health.backends.map((b) => {
                    const statusColor = STATUS_COLORS[b.status] ?? FALLBACK_STATUS_COLOR;
                    return (
                      <div key={b.name} className="dashboard-backend">
                        <span className="dashboard-backend__dot" style={{ backgroundColor: statusColor }} />
                        <span className="dashboard-backend__name">{b.name}</span>
                        <span className="dashboard-backend__info">{b.model_count} models · {b.protocol}</span>
                      </div>
                    );
                  })}
                </div>
                <div className="dashboard-health-stats">
                  <div><strong>{health.total_models}</strong> Models</div>
                  <div><strong>{health.total_agents}</strong> Agents</div>
                  <div><strong>{health.total_tools}</strong> Tools</div>
                  <div className="dashboard-config-source">Config: {health.config_source}</div>
                </div>
              </>
            )}
          </div>
        </div>
      </div>
    </div>
  );
}
