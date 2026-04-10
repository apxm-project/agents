import { useEffect, useState } from "react";
import { useAppStore } from "@/store/app-store";
import { fetchSessions, fetchSession, fetchSessionNode } from "@/api/session";
import { formatDuration, formatAbsoluteTimestamp, formatPreciseTimestamp, getErrorMessage, STATUS_COLORS, FALLBACK_STATUS_COLOR, statusBadgeStyle, stripNodePrefix, sessionPath as makeSessionPath } from "@/lib/format";
import { eventSummary } from "@/lib/event-utils";
import { EventFeed } from "@/components/shared/event-feed";
import type { ActivityKey } from "@/lib/constants";

export function SessionView() {
  const sessions = useAppStore((s) => s.sessions);
  const setSessions = useAppStore((s) => s.setSessions);
  const sessionData = useAppStore((s) => s.sessionData);
  const sessionPath = useAppStore((s) => s.sessionPath);
  const setSessionData = useAppStore((s) => s.setSessionData);
  const setGraphData = useAppStore((s) => s.setGraphData);
  const setActiveTab = useAppStore((s) => s.setActiveTab);
  const navigateToLive = useAppStore((s) => s.navigateToLive);
  const navigateToReplay = useAppStore((s) => s.navigateToReplay);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [traceOpen, setTraceOpen] = useState(false);
  const [expandedNodes, setExpandedNodes] = useState<Record<string, Record<string, unknown> | null>>({});
  const [loadingNode, setLoadingNode] = useState<string | null>(null);

  useEffect(() => {
    if (sessions.length === 0) {
      fetchSessions().then(setSessions).catch(() => {});
    }
  }, [sessions.length, setSessions]);

  async function handleSelectSession(path: string) {
    setLoading(true);
    setError(null);
    setTraceOpen(false);
    setExpandedNodes({});
    try {
      const data = await fetchSession(path);
      setSessionData(data, path);
    } catch (e) {
      setError(getErrorMessage(e));
    }
    setLoading(false);
  }

  async function handleNodeClick(key: string) {
    if (expandedNodes[key] !== undefined) {
      // Toggle collapse
      setExpandedNodes((prev) => {
        const next = { ...prev };
        delete next[key];
        return next;
      });
      return;
    }
    if (!sessionPath) return;
    // Extract node ID from key like "01_spawn_architect"
    const nodeId = key.match(/^(\d+)/)?.[1];
    if (!nodeId) return;
    setLoadingNode(key);
    try {
      const detail = await fetchSessionNode(nodeId, sessionPath);
      setExpandedNodes((prev) => ({ ...prev, [key]: detail }));
    } catch {
      setExpandedNodes((prev) => ({ ...prev, [key]: null }));
    }
    setLoadingNode(null);
  }

  function handleOpenGraph() {
    const graphName = sessionData?.manifest?.graph_name;
    if (!graphName) return;
    // Try to fetch the graph by name — the workflows endpoint gives us the path
    import("@/api/workflows").then(({ fetchWorkflows }) => {
      fetchWorkflows().then((wfs) => {
        const match = wfs.find((w) => w.name === graphName);
        if (match) {
          import("@/api/graph").then(({ fetchGraph }) => {
            fetchGraph(match.path).then((graph) => {
              setGraphData(graph, match.path);
              setActiveTab("graph" as ActivityKey);
            }).catch(() => {});
          });
        }
      }).catch(() => {});
    });
  }

  const sessionStatus = sessionData?.manifest?.status ?? "unknown";

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
                onClick={() => handleSelectSession(makeSessionPath(s.id))}
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
                <span style={statusBadgeStyle(STATUS_COLORS[sessionStatus] ?? FALLBACK_STATUS_COLOR)}>
                  {sessionStatus}
                </span>
                <div className="session-header__actions">
                  {sessionStatus === "running" && sessionPath ? (
                    <button type="button" className="session-action-btn session-action-btn--live" onClick={() => navigateToLive(sessionPath)}>
                      Watch Live
                    </button>
                  ) : null}
                  {(sessionStatus === "completed" || sessionStatus === "failed") && sessionPath ? (
                    <button type="button" className="session-action-btn" onClick={() => navigateToReplay(sessionPath)}>
                      Replay
                    </button>
                  ) : null}
                  {sessionData.manifest?.graph_name ? (
                    <button type="button" className="session-action-btn" onClick={handleOpenGraph}>
                      Open Graph
                    </button>
                  ) : null}
                </div>
              </div>

              {sessionData.manifest?.duration_ms != null ? (
                <div className="session-metrics">Duration: {formatDuration(sessionData.manifest.duration_ms)}</div>
              ) : null}

              {/* Metrics display */}
              {sessionData.metrics ? (
                <details className="node-detail-expand">
                  <summary className="trace-toggle"><strong>Metrics</strong></summary>
                  <pre className="session-json">{JSON.stringify(sessionData.metrics, null, 2)}</pre>
                </details>
              ) : null}

              {/* Results display */}
              {sessionData.results ? (
                <details className="node-detail-expand">
                  <summary className="trace-toggle"><strong>Results</strong></summary>
                  <pre className="session-json">{JSON.stringify(sessionData.results, null, 2)}</pre>
                </details>
              ) : null}

              {/* Trace section */}
              <div className="session-trace">
                <button type="button" className="trace-toggle" onClick={() => setTraceOpen(!traceOpen)}>
                  <strong>Trace events: {sessionData.trace.length}</strong>
                  <span>{traceOpen ? "\u25B4" : "\u25BE"}</span>
                </button>
                {traceOpen ? (
                  <EventFeed events={sessionData.trace} limit={200} style={{ maxHeight: "300px" }} />
                ) : null}
              </div>

              {/* Node statuses with expandable detail */}
              {sessionData.node_statuses ? (
                <div className="session-nodes">
                  <strong>Node Statuses:</strong>
                  <div className="node-status-grid">
                    {Object.entries(sessionData.node_statuses).map(([key, val]) => {
                      const status = (val as Record<string, unknown>)?.status as string ?? "unknown";
                      const color = STATUS_COLORS[status] ?? FALLBACK_STATUS_COLOR;
                      const isExpanded = expandedNodes[key] !== undefined;
                      const nodeDetail = expandedNodes[key];
                      return (
                        <div key={key}>
                          <button
                            type="button"
                            className={`node-status-cell node-status-cell--clickable${isExpanded ? " node-status-cell--expanded" : ""}`}
                            style={{ borderLeftColor: color }}
                            onClick={() => handleNodeClick(key)}
                          >
                            <div className="node-status-cell__name">
                              {stripNodePrefix(key)}
                              {loadingNode === key ? " ..." : null}
                            </div>
                            <div className="node-status-cell__status" style={statusBadgeStyle(color)}>{status}</div>
                          </button>
                          {isExpanded && nodeDetail ? (
                            <div className="node-detail-expand">
                              {nodeDetail.prompt ? (
                                <div className="node-detail-expand__section">
                                  <strong>Prompt</strong>
                                  <pre className="session-json">{String(nodeDetail.prompt)}</pre>
                                </div>
                              ) : null}
                              {nodeDetail.response ? (
                                <div className="node-detail-expand__section">
                                  <strong>Response</strong>
                                  <pre className="session-json">{String(nodeDetail.response)}</pre>
                                </div>
                              ) : null}
                              {nodeDetail.output ? (
                                <div className="node-detail-expand__section">
                                  <strong>Output</strong>
                                  <pre className="session-json">{JSON.stringify(nodeDetail.output, null, 2)}</pre>
                                </div>
                              ) : null}
                              {nodeDetail.trace ? (
                                <div className="node-detail-expand__section">
                                  <strong>Node Trace</strong>
                                  <div className="event-feed" style={{ maxHeight: "200px" }}>
                                    {(nodeDetail.trace as Array<Record<string, unknown>>).slice(-50).reverse().map((event, i) => (
                                      <div key={i} className="event-feed__item">
                                        <span className="event-feed__time">{formatPreciseTimestamp(String(event.timestamp ?? ""))}</span>
                                        <span className="event-feed__kind">{String(event.kind ?? "")}</span>
                                        <span className="event-feed__summary">{eventSummary(event as { kind: string; [key: string]: unknown })}</span>
                                      </div>
                                    ))}
                                  </div>
                                </div>
                              ) : null}
                            </div>
                          ) : null}
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
