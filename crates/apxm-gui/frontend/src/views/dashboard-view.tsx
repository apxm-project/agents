import { useEffect } from "react";
import { useAppStore } from "@/store/app-store";
import { fetchWorkflows } from "@/api/workflows";
import { fetchHealth } from "@/api/health";
import { fetchSessions } from "@/api/session";
import { formatDuration, formatAbsoluteTimestamp, STATUS_COLORS, FALLBACK_STATUS_COLOR, statusBadgeStyle } from "@/lib/format";
import type { ActivityKey } from "@/lib/constants";

export function DashboardView() {
  const workflows = useAppStore((s) => s.workflows);
  const sessions = useAppStore((s) => s.sessions);
  const health = useAppStore((s) => s.health);
  const setWorkflows = useAppStore((s) => s.setWorkflows);
  const setSessions = useAppStore((s) => s.setSessions);
  const setHealth = useAppStore((s) => s.setHealth);
  const setActiveTab = useAppStore((s) => s.setActiveTab);
  const setGraphData = useAppStore((s) => s.setGraphData);

  useEffect(() => {
    fetchWorkflows().then(setWorkflows).catch(() => {});
    fetchSessions().then(setSessions).catch(() => {});
    fetchHealth().then(setHealth).catch(() => {});
  }, [setWorkflows, setSessions, setHealth]);

  function handleWorkflowClick(path: string) {
    import("@/api/graph").then(({ fetchGraph }) => {
      fetchGraph(path).then((graph) => {
        setGraphData(graph, path);
        setActiveTab("graph" as ActivityKey);
      }).catch(() => {});
    });
  }

  function handleSessionClick() {
    setActiveTab("execution" as ActivityKey);
  }

  const recentSessions = sessions.slice(0, 10);

  return (
    <div className="view-panel">
      <div className="view-panel__header">
        <h2>Dashboard</h2>
      </div>
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
                <button
                  key={s.id}
                  type="button"
                  className="dashboard-session"
                  onClick={handleSessionClick}
                >
                  <div className="dashboard-session__name">{s.graph_name ?? s.id}</div>
                  <div className="dashboard-session__meta">
                    <span style={statusBadgeStyle(color)}>{s.status}</span>
                    <span>{formatAbsoluteTimestamp(s.started_at)}</span>
                    {(s as Record<string, unknown>).duration_ms != null ? (
                      <span>{formatDuration((s as Record<string, unknown>).duration_ms as number)}</span>
                    ) : null}
                  </div>
                </button>
              );
            })}
          </div>
        </div>

        {/* Health column */}
        <div className="dashboard-card">
          <div className="dashboard-card__header">
            <h3>Infrastructure</h3>
          </div>
          <div className="dashboard-card__body">
            {health == null ? (
              <div className="dashboard-empty">Loading health status...</div>
            ) : (
              <>
                <div className="dashboard-health-section">
                  <strong>Backends ({health.backends.length})</strong>
                  {health.backends.map((b) => {
                    const statusColor = b.status === "healthy" ? "var(--success)"
                      : b.status === "degraded" ? "var(--warning)"
                      : b.status === "unreachable" ? "var(--danger)"
                      : "var(--muted)";
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
