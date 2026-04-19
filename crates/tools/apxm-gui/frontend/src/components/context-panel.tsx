import { useEffect, useState, useCallback, useMemo, useRef } from "react";
import { useAppStore } from "@/store/app-store";
import { CtxTab, Overlay, CONTEXT_TABS } from "@/lib/constants";
import { fetchSession, fetchSessionNode } from "@/api/session";
import { fetchFile } from "@/api/file";
import { highlightLine } from "@/lib/syntax-highlight";
import {
  STATUS_COLORS,
  FALLBACK_STATUS_COLOR,
  statusBadgeStyle,
  formatDuration,
  formatPreciseTimestamp,
  timeAgo,
  humanizeNodeName,
  sessionPath,
} from "@/lib/format";
import { eventSummary } from "@/lib/event-utils";
import type { SessionData, SessionInfo, NodeStatusEntry } from "@/types/session";
import type { TraceEvent } from "@/types/events";
import type { ApxmGraph } from "@/types/graph";

const TABS = CONTEXT_TABS;

function normalizeNodeStatuses(
  raw: NodeStatusEntry[] | Record<string, unknown> | null,
): NodeStatusEntry[] {
  if (!raw) return [];
  if (Array.isArray(raw)) return raw as NodeStatusEntry[];
  return Object.entries(raw).map(([key, val], idx) => {
    const obj = val as Record<string, unknown>;
    const idMatch = key.match(/^(\d+)/);
    return {
      node_id: idMatch ? parseInt(idMatch[1], 10) : idx,
      status: String(obj?.status ?? "unknown"),
      retries: (obj?.retries as number) ?? 0,
      last_error: (obj?.last_error as string) ?? null,
      started_at_ms: obj?.started_at_ms as number | undefined,
      finished_at_ms: obj?.finished_at_ms as number | undefined,
      duration_ms: obj?.duration_ms as number | undefined,
    };
  });
}

export function ContextPanel() {
  const contextPanelTab = useAppStore((s) => s.contextPanelTab);
  const setContextPanelTab = useAppStore((s) => s.setContextPanelTab);
  const selectedWorkflow = useAppStore((s) => s.selectedWorkflow);
  const graphData = useAppStore((s) => s.graphData);
  const liveActive = useAppStore((s) => s.liveActive);
  const liveSessionPath = useAppStore((s) => s.liveSessionPath);
  const liveEvents = useAppStore((s) => s.liveEvents);
  const liveNodeStates = useAppStore((s) => s.liveNodeStates);
  const liveManifest = useAppStore((s) => s.liveManifest);
  const sessions = useAppStore((s) => s.sessions);
  const toggleContextPanel = useAppStore((s) => s.toggleContextPanel);
  const setOverlayView = useAppStore((s) => s.setOverlayView);
  const clearLive = useAppStore((s) => s.clearLive);

  const [sourceText, setSourceText] = useState<string | null>(null);
  const [selectedSession, setSelectedSession] = useState<SessionData | null>(null);
  const [loadingSession, setLoadingSession] = useState(false);
  const [expandedNodeId, setExpandedNodeId] = useState<string | null>(null);
  const [nodeDetail, setNodeDetail] = useState<Record<string, unknown> | null>(null);
  const [loadingNode, setLoadingNode] = useState(false);
  const [selectedSessionPath, setSelectedSessionPath] = useState<string | null>(null);

  // Auto-switch to live tab when execution starts
  useEffect(() => {
    if (liveActive && liveSessionPath) {
      setContextPanelTab(CtxTab.LIVE);
    }
  }, [liveActive, liveSessionPath, setContextPanelTab]);

  // Load source text when source tab selected and workflow is chosen
  useEffect(() => {
    if (contextPanelTab !== CtxTab.SOURCE || !selectedWorkflow) {
      return;
    }
    fetchFile(selectedWorkflow.path)
      .then(setSourceText)
      .catch(() => setSourceText(null));
  }, [contextPanelTab, selectedWorkflow]);

  // Filter sessions for selected workflow
  const workflowSessions = useMemo(() => {
    if (!selectedWorkflow) return sessions.slice(0, 10);
    return sessions.filter((s) => s.graph_name === selectedWorkflow.name).slice(0, 20);
  }, [sessions, selectedWorkflow]);

  const handleSessionSelect = useCallback((s: SessionInfo) => {
    const path = sessionPath(s.id);
    setSelectedSessionPath(path);
    setLoadingSession(true);
    setExpandedNodeId(null);
    setNodeDetail(null);
    fetchSession(path)
      .then(setSelectedSession)
      .catch(() => setSelectedSession(null))
      .finally(() => setLoadingSession(false));
  }, []);

  const handleNodeExpand = useCallback(async (nodeId: string) => {
    if (expandedNodeId === nodeId) {
      setExpandedNodeId(null);
      setNodeDetail(null);
      return;
    }
    if (!selectedSessionPath) return;
    setExpandedNodeId(nodeId);
    setLoadingNode(true);
    try {
      const detail = await fetchSessionNode(nodeId, selectedSessionPath);
      setNodeDetail(detail);
    } catch {
      setNodeDetail(null);
    }
    setLoadingNode(false);
  }, [expandedNodeId, selectedSessionPath]);

  // Live stats
  const liveStats = useMemo(() => {
    let c = 0, r = 0, f = 0;
    for (const status of Object.values(liveNodeStates)) {
      if (status === "completed") c++;
      else if (status === "running") r++;
      else if (status === "failed") f++;
    }
    const t = graphData?.nodes.length ?? Math.max(Object.keys(liveNodeStates).length, 1);
    return { completed: c, running: r, failed: f, total: t };
  }, [liveNodeStates, graphData]);

  const liveProgressPct = liveStats.total > 0
    ? Math.round((liveStats.completed / liveStats.total) * 100)
    : 0;

  return (
    <div className="ctx-panel">
      <div className="ctx-panel__header">
        <div className="ctx-panel__tabs">
          {TABS.map((t) => (
            <button
              key={t.key}
              type="button"
              className={`ctx-tab${contextPanelTab === t.key ? " ctx-tab--active" : ""}`}
              onClick={() => setContextPanelTab(t.key)}
            >
              {t.label}
              {t.key === CtxTab.LIVE && liveActive && (
                <span className="ctx-tab__live-dot" />
              )}
            </button>
          ))}
        </div>
        <button
          type="button"
          className="ctx-panel__close"
          onClick={toggleContextPanel}
          title="Close panel"
        >
          <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2">
            <path d="M18 6L6 18M6 6l12 12" />
          </svg>
        </button>
      </div>

      <div className="ctx-panel__body">
        {/* ── Graph Tab ── */}
        {contextPanelTab === CtxTab.GRAPH && (
          <GraphTabContent
            graphData={graphData}
            liveNodeStates={liveNodeStates}
            onOpenStudio={() => setOverlayView(Overlay.STUDIO)}
          />
        )}

        {/* ── Source Tab ── */}
        {contextPanelTab === CtxTab.SOURCE && (
          <SourceTabContent
            sourceText={sourceText}
            selectedWorkflow={selectedWorkflow}
          />
        )}

        {/* ── Sessions Tab ── */}
        {contextPanelTab === CtxTab.SESSIONS && (
          <SessionsTabContent
            sessions={workflowSessions}
            selectedSession={selectedSession}
            loadingSession={loadingSession}
            expandedNodeId={expandedNodeId}
            nodeDetail={nodeDetail}
            loadingNode={loadingNode}
            onSessionSelect={handleSessionSelect}
            onNodeExpand={handleNodeExpand}
            graphData={graphData}
          />
        )}

        {/* ── Live Tab ── */}
        {contextPanelTab === CtxTab.LIVE && (
          <LiveTabContent
            liveActive={liveActive}
            liveEvents={liveEvents}
            liveNodeStates={liveNodeStates}
            liveManifest={liveManifest}
            liveStats={liveStats}
            liveProgressPct={liveProgressPct}
            graphData={graphData}
            onDisconnect={clearLive}
          />
        )}
      </div>
    </div>
  );
}

// ─── Graph Tab ─────────────────────────────────────────────────────────────

function GraphTabContent({
  graphData,
  liveNodeStates,
  onOpenStudio,
}: {
  graphData: ApxmGraph | null;
  liveNodeStates: Record<number, string>;
  onOpenStudio: () => void;
}) {
  if (!graphData) {
    return (
      <div className="ctx-empty">
        <svg width="40" height="40" viewBox="0 0 24 24" fill="none" stroke="var(--ink-faint)" strokeWidth="1.2">
          <rect x="3" y="3" width="18" height="18" rx="2" />
          <circle cx="8.5" cy="8.5" r="1.5" />
          <circle cx="15.5" cy="8.5" r="1.5" />
          <circle cx="12" cy="15.5" r="1.5" />
          <path d="M9.5 9.5l2 4.5M14.5 9.5l-2 4.5" />
        </svg>
        <p>Select a workflow to preview its graph</p>
      </div>
    );
  }

  const cols = Math.min(4, Math.ceil(Math.sqrt(graphData.nodes.length)));

  return (
    <div className="ctx-graph">
      <div className="ctx-graph__info">
        <span className="ctx-graph__name">{graphData.name}</span>
        <span className="ctx-graph__stats">
          {graphData.nodes.length} nodes &middot; {graphData.edges.length} edges
        </span>
      </div>
      <div className="ctx-graph__grid" style={{ gridTemplateColumns: `repeat(${cols}, 1fr)` }}>
        {graphData.nodes.map((node) => {
          const liveStatus = liveNodeStates[node.id] as string | undefined;
          const color = liveStatus ? (STATUS_COLORS[liveStatus] ?? FALLBACK_STATUS_COLOR) : "var(--panel-border)";
          return (
            <div
              key={node.id}
              className={`ctx-graph__node${liveStatus ? ` ctx-graph__node--${liveStatus}` : ""}`}
              style={{ borderColor: color }}
              title={`#${node.id} ${node.name} (${node.op})`}
            >
              <span className="ctx-graph__node-name">{humanizeNodeName(node.name)}</span>
              <span className="ctx-graph__node-op">{node.op}</span>
              {liveStatus && (
                <span className="ctx-graph__node-badge" style={statusBadgeStyle(color)}>
                  {liveStatus}
                </span>
              )}
            </div>
          );
        })}
      </div>
      <button type="button" className="ctx-graph__studio-btn" onClick={onOpenStudio}>
        Open in Studio
      </button>
    </div>
  );
}

// ─── Source Tab ─────────────────────────────────────────────────────────────

function SourceTabContent({
  sourceText,
  selectedWorkflow,
}: {
  sourceText: string | null;
  selectedWorkflow: any;
}) {
  if (!selectedWorkflow) {
    return (
      <div className="ctx-empty">
        <p>Select a workflow to view its source</p>
      </div>
    );
  }

  if (sourceText == null) {
    return (
      <div className="ctx-empty">
        <div className="viewer-status__spinner" />
        <p>Loading source...</p>
      </div>
    );
  }

  const lines = sourceText.split("\n");
  const lang = selectedWorkflow.path?.endsWith(".py") ? "python" : "mlir";

  return (
    <div className="ctx-source">
      <div className="ctx-source__header">
        <span className="ctx-source__filename">
          {selectedWorkflow.path?.split("/").pop() ?? selectedWorkflow.name}
        </span>
        <span className="ctx-source__lines">{lines.length} lines</span>
      </div>
      <pre className="ctx-source__code">
        <code>
          {lines.map((line, i) => (
            <div key={i} className="ctx-source__line">
              <span className="ctx-source__lineno">{i + 1}</span>
              <span className="ctx-source__text">{highlightLine(line, lang)}</span>
            </div>
          ))}
        </code>
      </pre>
    </div>
  );
}

// ─── Sessions Tab ──────────────────────────────────────────────────────────

function SessionsTabContent({
  sessions,
  selectedSession,
  loadingSession,
  expandedNodeId,
  nodeDetail,
  loadingNode,
  onSessionSelect,
  onNodeExpand,
  graphData,
}: {
  sessions: SessionInfo[];
  selectedSession: SessionData | null;
  loadingSession: boolean;
  expandedNodeId: string | null;
  nodeDetail: Record<string, unknown> | null;
  loadingNode: boolean;
  onSessionSelect: (s: SessionInfo) => void;
  onNodeExpand: (nodeId: string) => void;
  graphData: ApxmGraph | null;
}) {
  if (sessions.length === 0) {
    return (
      <div className="ctx-empty">
        <p>No sessions yet. Execute a workflow to create one.</p>
      </div>
    );
  }

  const normalizedStatuses = selectedSession
    ? normalizeNodeStatuses(selectedSession.node_statuses)
    : [];

  const nodeNames = useMemo(() => {
    const map: Record<number, string> = {};
    if (selectedSession?.node_names) {
      for (const [idStr, name] of Object.entries(selectedSession.node_names)) {
        map[parseInt(idStr, 10)] = name;
      }
    }
    if (graphData) {
      for (const node of graphData.nodes) {
        if (!map[node.id]) map[node.id] = node.name;
      }
    }
    return map;
  }, [selectedSession?.node_names, graphData]);

  return (
    <div className="ctx-sessions">
      {/* Session list */}
      <div className="ctx-sessions__list">
        {sessions.map((s) => {
          const color = STATUS_COLORS[s.status] ?? FALLBACK_STATUS_COLOR;
          return (
            <button
              key={s.id}
              type="button"
              className="ctx-sessions__item"
              onClick={() => onSessionSelect(s)}
            >
              <span className="ctx-sessions__item-name">{s.graph_name ?? s.id}</span>
              <div className="ctx-sessions__item-meta">
                <span className="ctx-sessions__item-badge" style={statusBadgeStyle(color)}>
                  {s.status}
                </span>
                <span>{s.started_at ? timeAgo(s.started_at) : ""}</span>
                {s.duration_ms != null && <span>{formatDuration(s.duration_ms)}</span>}
              </div>
            </button>
          );
        })}
      </div>

      {/* Selected session detail */}
      {loadingSession && (
        <div className="ctx-empty"><div className="viewer-status__spinner" /> Loading...</div>
      )}
      {selectedSession && !loadingSession && (
        <div className="ctx-sessions__detail">
          <div className="ctx-sessions__detail-header">
            <span className="ctx-sessions__detail-name">
              {selectedSession.manifest?.graph_name ?? "Session"}
            </span>
            <span
              className="ctx-sessions__detail-badge"
              style={statusBadgeStyle(STATUS_COLORS[selectedSession.manifest?.status ?? "unknown"] ?? FALLBACK_STATUS_COLOR)}
            >
              {selectedSession.manifest?.status ?? "unknown"}
            </span>
            {selectedSession.manifest?.duration_ms != null && (
              <span className="ctx-sessions__detail-dur">
                {formatDuration(selectedSession.manifest.duration_ms)}
              </span>
            )}
          </div>

          {/* Nodes */}
          {normalizedStatuses.length > 0 && (
            <div className="ctx-sessions__nodes">
              {[...normalizedStatuses]
                .sort((a, b) => {
                  if (a.started_at_ms != null && b.started_at_ms != null) return a.started_at_ms - b.started_at_ms;
                  return a.node_id - b.node_id;
                })
                .map((ns) => {
                  const status = ns.status.toLowerCase();
                  const color = STATUS_COLORS[status] ?? FALLBACK_STATUS_COLOR;
                  const nodeId = String(ns.node_id);
                  const name = nodeNames[ns.node_id] ?? `node_${ns.node_id}`;
                  const isExpanded = expandedNodeId === nodeId;

                  return (
                    <div key={ns.node_id} className="ctx-node-item">
                      <button
                        type="button"
                        className={`ctx-node-item__header${isExpanded ? " ctx-node-item__header--expanded" : ""}`}
                        style={{ borderLeftColor: color }}
                        onClick={() => onNodeExpand(nodeId)}
                      >
                        <span className="ctx-node-item__name">{humanizeNodeName(name)}</span>
                        <div className="ctx-node-item__meta">
                          {ns.duration_ms != null && (
                            <span className="ctx-node-item__dur">{formatDuration(ns.duration_ms)}</span>
                          )}
                          <span className="ctx-node-item__badge" style={statusBadgeStyle(color)}>
                            {status}
                          </span>
                        </div>
                      </button>
                      {isExpanded && nodeDetail && (
                        <div className="ctx-node-detail">
                          {ns.last_error && (
                            <div className="ctx-node-detail__section ctx-node-detail__section--error">
                              <strong>Error</strong>
                              <pre>{ns.last_error}</pre>
                            </div>
                          )}
                          {(nodeDetail.prompt as string) && (
                            <div className="ctx-node-detail__section">
                              <strong>Prompt</strong>
                              <pre>{nodeDetail.prompt as string}</pre>
                            </div>
                          )}
                          {(nodeDetail.response as string) && (
                            <div className="ctx-node-detail__section">
                              <strong>Response</strong>
                              <pre>{nodeDetail.response as string}</pre>
                            </div>
                          )}
                          {nodeDetail.output != null && (
                            <div className="ctx-node-detail__section">
                              <strong>Output</strong>
                              <pre>{JSON.stringify(nodeDetail.output, null, 2)}</pre>
                            </div>
                          )}
                          {loadingNode && <div className="ctx-node-detail__loading">Loading...</div>}
                        </div>
                      )}
                    </div>
                  );
                })}
            </div>
          )}

          {/* Results */}
          {selectedSession.results && (
            <details className="ctx-sessions__results">
              <summary>Results</summary>
              <pre className="ctx-json">{JSON.stringify(selectedSession.results, null, 2)}</pre>
            </details>
          )}

          {/* Metrics */}
          {selectedSession.metrics && (
            <details className="ctx-sessions__metrics">
              <summary>Metrics</summary>
              <pre className="ctx-json">{JSON.stringify(selectedSession.metrics, null, 2)}</pre>
            </details>
          )}
        </div>
      )}
    </div>
  );
}

// ─── Live Tab ──────────────────────────────────────────────────────────────

function LiveTabContent({
  liveActive,
  liveEvents,
  liveNodeStates,
  liveManifest,
  liveStats,
  liveProgressPct,
  graphData,
  onDisconnect,
}: {
  liveActive: boolean;
  liveEvents: TraceEvent[];
  liveNodeStates: Record<number, string>;
  liveManifest: Record<string, unknown> | null;
  liveStats: { completed: number; running: number; failed: number; total: number };
  liveProgressPct: number;
  graphData: ApxmGraph | null;
  onDisconnect: () => void;
}) {
  const eventEndRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    eventEndRef.current?.scrollIntoView({ behavior: "smooth", block: "end" });
  }, [liveEvents.length]);

  const nodeNames = useMemo(() => {
    const map: Record<number, string> = {};
    if (graphData) {
      for (const node of graphData.nodes) map[node.id] = node.name;
    }
    return map;
  }, [graphData]);

  const liveNodeEntries = useMemo(() => {
    const entries = Object.entries(liveNodeStates) as [string, string][];
    const order: Record<string, number> = { running: 0, pending: 1, failed: 2, completed: 3 };
    return entries.sort(([, a], [, b]) => (order[a] ?? 9) - (order[b] ?? 9));
  }, [liveNodeStates]);

  const manifestStatus = (liveManifest?.status as string) ?? (liveActive ? "running" : "idle");
  const elapsed = liveManifest?.duration_ms as number | undefined;

  if (!liveActive && liveEvents.length === 0) {
    return (
      <div className="ctx-empty">
        <svg width="40" height="40" viewBox="0 0 24 24" fill="none" stroke="var(--ink-faint)" strokeWidth="1.2">
          <circle cx="12" cy="12" r="10" />
          <path d="M10 8l6 4-6 4V8z" />
        </svg>
        <p>No active execution. Run a workflow to see live progress.</p>
      </div>
    );
  }

  // Gather token output
  const tokenOutput = useMemo(() => {
    return liveEvents
      .filter((e) => e.kind === "token" && typeof (e as Record<string, unknown>).text === "string")
      .map((e) => (e as Record<string, unknown>).text as string)
      .join("");
  }, [liveEvents]);

  return (
    <div className="ctx-live">
      {/* Header */}
      <div className="ctx-live__header">
        <span className="ctx-live__status" style={statusBadgeStyle(STATUS_COLORS[manifestStatus] ?? FALLBACK_STATUS_COLOR)}>
          {manifestStatus === "running" && <span className="ctx-live__pulse" />}
          {manifestStatus}
        </span>
        {elapsed != null && elapsed > 0 && (
          <span className="ctx-live__elapsed">{formatDuration(elapsed)}</span>
        )}
        {liveActive && (
          <button type="button" className="ghost-button ghost-button--sm" onClick={onDisconnect}>
            Disconnect
          </button>
        )}
      </div>

      {/* Progress bar */}
      <div className="ctx-live__progress">
        <div className="ctx-live__progress-info">
          <span>{liveStats.completed}/{liveStats.total} nodes</span>
          {liveStats.running > 0 && <span> &middot; {liveStats.running} running</span>}
          {liveStats.failed > 0 && <span> &middot; {liveStats.failed} failed</span>}
        </div>
        <div className="ctx-live__track">
          <div className="ctx-live__fill" style={{ width: `${liveProgressPct}%` }} />
          {liveStats.running > 0 && (
            <div
              className="ctx-live__fill ctx-live__fill--running"
              style={{
                width: `${Math.round((liveStats.running / liveStats.total) * 100)}%`,
                left: `${liveProgressPct}%`,
              }}
            />
          )}
        </div>
        <span className="ctx-live__pct">{liveProgressPct}%</span>
      </div>

      {/* Node status grid */}
      {liveNodeEntries.length > 0 && (
        <div className="ctx-live__nodes">
          {liveNodeEntries.map(([id, status]) => {
            const color = STATUS_COLORS[status] ?? FALLBACK_STATUS_COLOR;
            const name = nodeNames[Number(id)] ?? `Node #${id}`;
            return (
              <div key={id} className="ctx-live__node" style={{ borderLeftColor: color }}>
                <span className="ctx-live__node-name">{humanizeNodeName(name)}</span>
                <span className="ctx-live__node-badge" style={statusBadgeStyle(color)}>
                  {status === "running" && <span className="ctx-live__node-pulse" />}
                  {status}
                </span>
              </div>
            );
          })}
        </div>
      )}

      {/* Token output */}
      {tokenOutput && (
        <details className="ctx-live__output" open>
          <summary>Output</summary>
          <pre className="ctx-live__output-pre">{tokenOutput}</pre>
        </details>
      )}

      {/* Recent events */}
      <div className="ctx-live__events">
        <span className="ctx-live__events-title">Events ({liveEvents.length})</span>
        {liveEvents.slice(-50).map((event, i) => (
          <div key={i} className={`ctx-live__event${event.kind === "operation_error" ? " ctx-live__event--error" : ""}`}>
            <span className="ctx-live__event-time">{formatPreciseTimestamp(event.timestamp)}</span>
            <span className="ctx-live__event-kind">{event.kind.replace(/_/g, " ")}</span>
            <span className="ctx-live__event-summary">{eventSummary(event)}</span>
          </div>
        ))}
        <div ref={eventEndRef} />
      </div>
    </div>
  );
}
