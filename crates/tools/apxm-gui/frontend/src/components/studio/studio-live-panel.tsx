import { useMemo, useRef, useEffect, useState, useCallback } from "react";
import { useAppStore } from "@/store/app-store";
import { fetchSessionNode } from "@/api/session";
import {
  STATUS_COLORS,
  FALLBACK_STATUS_COLOR,
  statusBadgeStyle,
  formatDuration,
  formatPreciseTimestamp,
} from "@/lib/format";
import { eventSummary } from "@/lib/event-utils";
import { CtxTab } from "@/lib/constants";
import type { TraceEvent } from "@/types/events";
import { TimelineView } from "./timeline-view";

type PanelTab = "events" | "timeline" | "nodes" | "detail";

export function StudioLivePanel() {
  const liveActive = useAppStore((s) => s.liveActive);
  const liveEvents = useAppStore((s) => s.liveEvents);
  const liveNodeStates = useAppStore((s) => s.liveNodeStates);
  const liveManifest = useAppStore((s) => s.liveManifest);
  const graphData = useAppStore((s) => s.graphData);
  const inlineRun = useAppStore((s) => s.inlineRun);
  const lastRunStatus = useAppStore((s) => s.lastRunStatus);
  const lastRunSessionPath = useAppStore((s) => s.lastRunSessionPath);
  const setContextPanelTab = useAppStore((s) => s.setContextPanelTab);
  const clearLive = useAppStore((s) => s.clearLive);
  const navigateToSession = useAppStore((s) => s.navigateToSession);
  const setSelectedNodeId = useAppStore((s) => s.setSelectedNodeId);

  const [collapsed, setCollapsed] = useState(false);
  const [tab, setTab] = useState<PanelTab>("events");
  const [detailNodeId, setDetailNodeId] = useState<number | null>(null);
  const [nodeDetail, setNodeDetail] = useState<Record<string, unknown> | null>(null);
  const [loadingDetail, setLoadingDetail] = useState(false);
  const [timelineNowMs, setTimelineNowMs] = useState(() => Date.now());
  const eventEndRef = useRef<HTMLDivElement>(null);

  const isLive = liveActive && inlineRun;
  const isFinished = !liveActive && lastRunStatus != null && inlineRun;

  // Auto-scroll events to bottom
  useEffect(() => {
    if (tab === "events" && eventEndRef.current && !collapsed) {
      eventEndRef.current.scrollIntoView({ behavior: "smooth", block: "end" });
    }
  }, [liveEvents.length, tab, collapsed]);

  // Build node name lookup
  const nodeNames = useMemo(() => {
    const map: Record<number, string> = {};
    if (graphData) {
      for (const node of graphData.nodes) {
        map[node.id] = node.name;
      }
    }
    return map;
  }, [graphData]);

  useEffect(() => {
    setTimelineNowMs(Date.now());
  }, [liveEvents.length, isFinished]);

  useEffect(() => {
    if (!isLive) {
      return;
    }
    const timer = window.setInterval(() => {
      setTimelineNowMs(Date.now());
    }, 250);
    return () => window.clearInterval(timer);
  }, [isLive]);

  // Progress stats
  const { completed, running, failed, total } = useMemo(() => {
    let c = 0, r = 0, f = 0;
    for (const status of Object.values(liveNodeStates)) {
      if (status === "completed") c++;
      else if (status === "running") r++;
      else if (status === "failed") f++;
    }
    const t = graphData?.nodes.length ?? Math.max(Object.keys(liveNodeStates).length, 1);
    return { completed: c, running: r, failed: f, total: t };
  }, [liveNodeStates, graphData]);

  const progressPct = total > 0 ? Math.round((completed / total) * 100) : 0;
  const manifestStatus = (liveManifest?.status as string) ?? (liveActive ? "running" : lastRunStatus ?? "idle");
  const statusColor = STATUS_COLORS[manifestStatus] ?? FALLBACK_STATUS_COLOR;
  const elapsed = liveManifest?.duration_ms as number | undefined;

  // Sort nodes: running first, then pending, then failed, then completed
  const sortedNodes = useMemo(() => {
    const entries = Object.entries(liveNodeStates) as [string, string][];
    const order: Record<string, number> = { running: 0, pending: 1, failed: 2, completed: 3 };
    return entries.sort(([, a], [, b]) => (order[a] ?? 9) - (order[b] ?? 9));
  }, [liveNodeStates]);

  // Extract token events for inline display
  const recentTokens = useMemo(() => {
    const tokens: { nodeId: number; text: string; timestamp: string }[] = [];
    const seen = liveEvents.slice(-200);
    for (const e of seen) {
      if (e.kind === "token" && typeof (e as Record<string, unknown>).text === "string") {
        tokens.push({
          nodeId: (e as Record<string, unknown>).node_id as number ?? 0,
          text: (e as Record<string, unknown>).text as string,
          timestamp: e.timestamp,
        });
      }
    }
    return tokens;
  }, [liveEvents]);

  const handleNodeClick = useCallback(async (nodeId: number) => {
    setSelectedNodeId(nodeId);
    if (!isFinished || !lastRunSessionPath) {
      setTab("nodes");
      return;
    }
    setDetailNodeId(nodeId);
    setTab("detail");
    setLoadingDetail(true);
    try {
      const detail = await fetchSessionNode(String(nodeId), lastRunSessionPath);
      setNodeDetail(detail);
    } catch {
      setNodeDetail(null);
    }
    setLoadingDetail(false);
  }, [isFinished, lastRunSessionPath, setSelectedNodeId]);

  const handleDismiss = useCallback(() => {
    clearLive();
    useAppStore.setState({ lastRunSessionPath: null, lastRunStatus: null, inlineRun: false });
  }, [clearLive]);

  if (!isLive && !isFinished) return null;

  return (
    <div className={`slp${isFinished ? ` slp--${manifestStatus}` : ""}`}>
      {/* Header bar — always visible */}
      <div className="slp__header" onClick={() => setCollapsed(!collapsed)}>
        <div className="slp__status-row">
          <span className="slp__badge" style={statusBadgeStyle(statusColor)}>
            {isLive && <span className="slp__pulse" />}
            {manifestStatus}
          </span>
          <span className="slp__progress-text">
            {completed}/{total} nodes
            {running > 0 && <> · {running} running</>}
            {failed > 0 && <> · {failed} failed</>}
          </span>
          {elapsed != null && elapsed > 0 && (
            <span className="slp__elapsed">{formatDuration(elapsed)}</span>
          )}
          <div className="slp__track">
            <div className="slp__fill" style={{ width: `${progressPct}%` }} />
            {running > 0 && (
              <div
                className="slp__fill slp__fill--running"
                style={{ width: `${Math.round((running / total) * 100)}%`, left: `${progressPct}%` }}
              />
            )}
          </div>
        </div>
        <div className="slp__actions">
          {isFinished && lastRunSessionPath && (
            <button type="button" className="ghost-button ghost-button--sm"
              onClick={(e) => { e.stopPropagation(); navigateToSession(lastRunSessionPath); }}>
              View Details
            </button>
          )}
          <button type="button" className="ghost-button ghost-button--sm"
            onClick={(e) => { e.stopPropagation(); setContextPanelTab(CtxTab.SESSIONS); }}>
            Full View
          </button>
          {isFinished && (
            <button type="button" className="ghost-button ghost-button--sm"
              onClick={(e) => { e.stopPropagation(); handleDismiss(); }}>
              Dismiss
            </button>
          )}
          <span className="slp__chevron">{collapsed ? "\u25BE" : "\u25B4"}</span>
        </div>
      </div>

      {/* Body — collapsible */}
      {!collapsed && (
        <div className="slp__body">
          {/* Tab bar */}
          <div className="slp__tabs">
            <button type="button"
              className={`slp__tab${tab === "events" ? " slp__tab--active" : ""}`}
              onClick={() => setTab("events")}>
              Events ({liveEvents.length})
            </button>
            <button type="button"
              className={`slp__tab${tab === "timeline" ? " slp__tab--active" : ""}`}
              onClick={() => setTab("timeline")}>
              Timeline
            </button>
            <button type="button"
              className={`slp__tab${tab === "nodes" ? " slp__tab--active" : ""}`}
              onClick={() => setTab("nodes")}>
              Nodes ({Object.keys(liveNodeStates).length})
            </button>
            {isFinished && (
              <button type="button"
                className={`slp__tab${tab === "detail" ? " slp__tab--active" : ""}`}
                onClick={() => setTab("detail")}>
                Detail
              </button>
            )}
          </div>

          {/* Events tab */}
          {tab === "events" && (
            <div className="slp__events">
              {liveEvents.length === 0 && (
                <div className="slp__empty">Waiting for events...</div>
              )}
              {liveEvents.slice(-150).map((event, i) => (
                <EventRow key={i} event={event} nodeNames={nodeNames}
                  onNodeClick={handleNodeClick} />
              ))}
              {recentTokens.length > 0 && (
                <div className="slp__tokens">
                  <span className="slp__tokens-label">LLM Output</span>
                  <pre className="slp__tokens-content">
                    {recentTokens.slice(-20).map((t) => t.text).join("")}
                  </pre>
                </div>
              )}
              <div ref={eventEndRef} />
            </div>
          )}

          {tab === "timeline" && (
            <TimelineView
              events={liveEvents}
              liveNodeStates={liveNodeStates}
              nodeNames={nodeNames}
              nowMs={timelineNowMs}
              onNodeClick={handleNodeClick}
            />
          )}

          {/* Nodes tab */}
          {tab === "nodes" && (
            <div className="slp__nodes">
              {sortedNodes.length === 0 && (
                <div className="slp__empty">No nodes tracked yet...</div>
              )}
              {sortedNodes.map(([id, status]) => {
                const color = STATUS_COLORS[status] ?? FALLBACK_STATUS_COLOR;
                const name = nodeNames[Number(id)] ?? `Node #${id}`;
                return (
                  <button key={id} type="button"
                    className={`slp__node slp__node--${status}`}
                    style={{ borderLeftColor: color }}
                    onClick={() => handleNodeClick(Number(id))}>
                    <span className="slp__node-name">{name}</span>
                    <span className="slp__node-id">#{id}</span>
                    <span className="slp__node-status" style={statusBadgeStyle(color)}>
                      {status === "running" && <span className="slp__pulse slp__pulse--sm" />}
                      {status}
                    </span>
                  </button>
                );
              })}
            </div>
          )}

          {/* Detail tab (post-execution) */}
          {tab === "detail" && (
            <div className="slp__detail">
              {detailNodeId == null && (
                <div className="slp__empty">Click a node to inspect its prompt, response, and output.</div>
              )}
              {detailNodeId != null && loadingDetail && (
                <div className="slp__empty">Loading node #{detailNodeId}...</div>
              )}
              {detailNodeId != null && !loadingDetail && nodeDetail && (() => {
                const prompt = nodeDetail.prompt as string | undefined;
                const response = nodeDetail.response as string | undefined;
                const output = nodeDetail.output;
                const name = nodeNames[detailNodeId] ?? `Node #${detailNodeId}`;
                return (
                  <>
                    <div className="slp__detail-header">
                      <strong>{name}</strong>
                      <span className="slp__detail-id">#{detailNodeId}</span>
                    </div>
                    {prompt && (
                      <div className="slp__detail-section">
                        <span className="slp__detail-label">Prompt</span>
                        <pre className="slp__detail-pre">{prompt}</pre>
                      </div>
                    )}
                    {response && (
                      <div className="slp__detail-section">
                        <span className="slp__detail-label">Response</span>
                        <pre className="slp__detail-pre">{response}</pre>
                      </div>
                    )}
                    {output != null && (
                      <div className="slp__detail-section">
                        <span className="slp__detail-label">Output</span>
                        <pre className="slp__detail-pre">{JSON.stringify(output, null, 2)}</pre>
                      </div>
                    )}
                    {!prompt && !response && output == null && (
                      <div className="slp__empty">No prompt/response data available for this node.</div>
                    )}
                  </>
                );
              })()}
            </div>
          )}
        </div>
      )}
    </div>
  );
}

function EventRow({ event, nodeNames, onNodeClick }: {
  event: TraceEvent;
  nodeNames: Record<number, string>;
  onNodeClick: (id: number) => void;
}) {
  const nodeId = (event as Record<string, unknown>).node_id as number | undefined;
  const nodeName = nodeId != null ? nodeNames[nodeId] : undefined;
  const isToken = event.kind === "token";
  const isError = event.kind === "operation_error" || event.kind === "session_error";

  return (
    <div className={`slp__event${isError ? " slp__event--error" : ""}${isToken ? " slp__event--token" : ""}`}>
      <span className="slp__event-time">{formatPreciseTimestamp(event.timestamp)}</span>
      <span className={`slp__event-kind slp__event-kind--${event.kind.split("_")[0]}`}>
        {event.kind.replace(/_/g, " ")}
      </span>
      {nodeId != null && (
        <button type="button" className="slp__event-node" onClick={() => onNodeClick(nodeId)}>
          {nodeName ?? `#${nodeId}`}
        </button>
      )}
      <span className="slp__event-summary">{eventSummary(event)}</span>
    </div>
  );
}
