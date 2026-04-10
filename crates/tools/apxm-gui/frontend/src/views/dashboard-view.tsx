import { useEffect, useRef, useState, useCallback } from "react";
import { useAppStore } from "@/store/app-store";
import { fetchWorkflows } from "@/api/workflows";
import { fetchHealth } from "@/api/health";
import { fetchSession, fetchSessions } from "@/api/session";
import { apiFetch } from "@/api/client";
import { formatDuration, formatAbsoluteTimestamp, STATUS_COLORS, FALLBACK_STATUS_COLOR, statusBadgeStyle, sessionPath } from "@/lib/format";
import type { ActivityKey } from "@/lib/constants";

type ExampleEntry = {
  path: string;
  relative_path: string;
  name: string;
  category: string;
  node_count: number | null;
};

function AnimatedCounter({ value, label }: { value: number; label: string }) {
  const [display, setDisplay] = useState(0);
  useEffect(() => {
    if (value === 0) { setDisplay(0); return; }
    const duration = 600;
    const start = performance.now();
    let raf: number;
    function tick(now: number) {
      const elapsed = now - start;
      const progress = Math.min(elapsed / duration, 1);
      const eased = 1 - Math.pow(1 - progress, 3);
      setDisplay(Math.round(eased * value));
      if (progress < 1) raf = requestAnimationFrame(tick);
    }
    raf = requestAnimationFrame(tick);
    return () => cancelAnimationFrame(raf);
  }, [value]);

  return (
    <div className="hero-stat">
      <span className="hero-stat__value">{display}</span>
      <span className="hero-stat__label">{label}</span>
    </div>
  );
}

function QuickAction({ icon, label, sublabel, onClick }: { icon: string; label: string; sublabel: string; onClick: () => void }) {
  return (
    <button type="button" className="quick-action" onClick={onClick}>
      <span className="quick-action__icon">{icon}</span>
      <span className="quick-action__text">
        <span className="quick-action__label">{label}</span>
        <span className="quick-action__sublabel">{sublabel}</span>
      </span>
    </button>
  );
}

export function DashboardView() {
  const workflows = useAppStore((s) => s.workflows);
  const ops = useAppStore((s) => s.ops);
  const passes = useAppStore((s) => s.passes);
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
  const [fetchErrors, setFetchErrors] = useState<Record<string, string>>({});
  const [examples, setExamples] = useState<ExampleEntry[]>([]);
  const [examplesFilter, setExamplesFilter] = useState("");
  const intervalRef = useRef<ReturnType<typeof setInterval> | null>(null);

  const addFetchError = useCallback((key: string, msg: string) => {
    setFetchErrors((prev) => ({ ...prev, [key]: msg }));
  }, []);

  const clearFetchError = useCallback((key: string) => {
    setFetchErrors((prev) => {
      const next = { ...prev };
      delete next[key];
      return next;
    });
  }, []);

  useEffect(() => {
    fetchWorkflows()
      .then((w) => { setWorkflows(w); clearFetchError("workflows"); })
      .catch((e) => addFetchError("workflows", `Workflows: ${e.message ?? e}`));

    fetchSessions()
      .then((s) => { setSessions(s); setLastUpdated(new Date()); clearFetchError("sessions"); })
      .catch((e) => addFetchError("sessions", `Sessions: ${e.message ?? e}`));

    fetchHealth()
      .then((h) => { setHealth(h); clearFetchError("health"); })
      .catch((e) => addFetchError("health", `Health: ${e.message ?? e}`));

    apiFetch<{ examples: ExampleEntry[] }>("/api/examples")
      .then((r) => { setExamples(r.examples); clearFetchError("examples"); })
      .catch((e) => addFetchError("examples", `Examples: ${e.message ?? e}`));
  }, [setWorkflows, setSessions, setHealth, addFetchError, clearFetchError]);

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
  const totalNodes = workflows.reduce((sum, w) => sum + (w.node_count ?? 0), 0);

  const filteredExamples = examplesFilter
    ? examples.filter((e) => {
        const q = examplesFilter.toLowerCase();
        return e.name.toLowerCase().includes(q) || e.category.toLowerCase().includes(q);
      })
    : examples;

  const exampleCategories = [...new Set(filteredExamples.map((e) => e.category))].sort();

  const errorEntries = Object.entries(fetchErrors);

  return (
    <div className="view-panel">
      {/* Hero section */}
      <div className="dashboard-hero">
        <div className="dashboard-hero__text">
          <h2 className="dashboard-hero__title">APXM Dashboard</h2>
          <p className="dashboard-hero__subtitle">Agent workflow compiler &amp; runtime</p>
        </div>
        <div className="dashboard-hero__stats">
          <AnimatedCounter value={workflows.length} label="workflows" />
          <AnimatedCounter value={totalNodes} label="total nodes" />
          <AnimatedCounter value={ops.length} label="AIS ops" />
          <AnimatedCounter value={passes.length} label="passes" />
          <AnimatedCounter value={sessions.length} label="sessions" />
          <AnimatedCounter value={health?.total_models ?? 0} label="models" />
        </div>
        <div className="dashboard-hero__actions">
          <QuickAction icon="⬡" label="Explorer" sublabel="Browse graphs" onClick={() => setActiveTab("graph" as ActivityKey)} />
          <QuickAction icon="◎" label="Reference" sublabel="AIS operations" onClick={() => setActiveTab("reference" as ActivityKey)} />
          <QuickAction icon="▶" label="Execute" sublabel="Run workflows" onClick={() => setActiveTab("execution" as ActivityKey)} />
          <QuickAction icon="⚙" label="Settings" sublabel="Configuration" onClick={() => setActiveTab("settings" as ActivityKey)} />
        </div>
        {lastUpdated ? (
          <span className="dashboard-hero__updated">
            Updated {lastUpdated.toLocaleTimeString()}
          </span>
        ) : null}
      </div>

      {/* Connection error banner */}
      {errorEntries.length > 0 ? (
        <div className="dashboard-errors">
          {errorEntries.map(([key, msg]) => (
            <div key={key} className="dashboard-error-item">
              <span className="dashboard-error-item__icon">⚠</span>
              <span>{msg}</span>
            </div>
          ))}
        </div>
      ) : null}

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
              <div className="dashboard-empty">
                {fetchErrors.workflows
                  ? "Could not load workflows. Is the API server running?"
                  : "No workflows found in project directory."}
              </div>
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
              <div className="dashboard-empty">
                {fetchErrors.sessions
                  ? "Could not load sessions."
                  : "No sessions yet. Execute a workflow to create one."}
              </div>
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
              {probing ? "Probing..." : "Probe"}
            </button>
          </div>
          <div className="dashboard-card__body">
            {health == null ? (
              <div className="dashboard-empty">
                {fetchErrors.health
                  ? "Could not reach backend."
                  : "Loading health status..."}
              </div>
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

      {/* Examples browser */}
      {examples.length > 0 ? (
        <div className="dashboard-examples">
          <div className="dashboard-examples__header">
            <h3>Example Graphs</h3>
            <input
              type="text"
              className="dashboard-examples__search"
              placeholder="Filter examples..."
              value={examplesFilter}
              onChange={(e) => setExamplesFilter(e.target.value)}
            />
            <span className="dashboard-card__count">{filteredExamples.length}</span>
          </div>
          <div className="dashboard-examples__body">
            {exampleCategories.map((cat) => {
              const catExamples = filteredExamples.filter((e) => e.category === cat);
              if (catExamples.length === 0) return null;
              return (
                <div key={cat} className="dashboard-examples__category">
                  <span className="dashboard-examples__category-label">{cat}</span>
                  <div className="dashboard-examples__items">
                    {catExamples.map((e) => (
                      <button
                        key={e.path}
                        type="button"
                        className="dashboard-examples__item"
                        onClick={() => handleWorkflowClick(e.path)}
                        title={e.relative_path}
                      >
                        <span className="dashboard-examples__item-name">{e.name}</span>
                        {e.node_count != null ? (
                          <span className="dashboard-examples__item-nodes">{e.node_count}n</span>
                        ) : null}
                      </button>
                    ))}
                  </div>
                </div>
              );
            })}
          </div>
        </div>
      ) : null}
    </div>
  );
}
