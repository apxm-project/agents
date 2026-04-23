import { useMemo } from "react";
import * as EK from "@/lib/event-kinds";
import {
  FALLBACK_STATUS_COLOR,
  STATUS_COLORS,
  formatDuration,
  formatPreciseTimestamp,
  statusBadgeStyle,
} from "@/lib/format";
import { eventSummary } from "@/lib/event-utils";
import type { NodeLiveStatus, TraceEvent } from "@/types/events";

type TimelineViewProps = {
  events: TraceEvent[];
  liveNodeStates: Record<number, NodeLiveStatus>;
  nodeNames: Record<number, string>;
  nowMs: number;
  onNodeClick: (id: number) => void;
};

type TimelineSegmentStatus = Extract<NodeLiveStatus, "running" | "completed" | "failed">;

type TimelineSegment = {
  nodeId: number;
  opType: string;
  startMs: number;
  endMs: number;
  startedAt: string;
  endedAt: string | null;
  status: TimelineSegmentStatus;
};

type TimelineMarkerKind = typeof EK.SCHEDULER_DECISION.name | typeof EK.HEAD_OF_LINE_BLOCK.name;

type TimelineMarker = {
  nodeId: number;
  atMs: number;
  atTimestamp: string;
  kind: TimelineMarkerKind;
  label: string;
  summary: string;
};

type TimelineLane = {
  nodeId: number;
  name: string;
  status: NodeLiveStatus;
  firstMs: number;
  segments: TimelineSegment[];
  markers: TimelineMarker[];
};

type TimelineModel = {
  lanes: TimelineLane[];
  rangeStartMs: number;
  rangeEndMs: number;
  rangeMs: number;
  runningCount: number;
  decisionCount: number;
  holCount: number;
};

const MIN_SEGMENT_WIDTH_PCT = 0.8;
const TICK_COUNT = 5;

export function TimelineView({
  events,
  liveNodeStates,
  nodeNames,
  nowMs,
  onNodeClick,
}: TimelineViewProps) {
  const model = useMemo(
    () => buildTimelineModel(events, liveNodeStates, nodeNames, nowMs),
    [events, liveNodeStates, nodeNames, nowMs],
  );

  if (model == null || model.lanes.length === 0) {
    return (
      <div className="slp__timeline">
        <div className="slp__empty">
          Timeline will appear once operation lifecycle events start arriving.
        </div>
      </div>
    );
  }

  const tickMarks = Array.from({ length: TICK_COUNT }, (_, index) => {
    const pct = (index / (TICK_COUNT - 1)) * 100;
    const offsetMs = Math.round((model.rangeMs * pct) / 100);
    return { pct, label: `+${formatDuration(offsetMs)}` };
  });

  return (
    <div className="slp__timeline">
      <div className="slp__timeline-meta">
        <span className="slp__timeline-chip">
          Window {formatDuration(model.rangeMs)}
        </span>
        <span className="slp__timeline-chip">
          {model.lanes.length} nodes
        </span>
        <span className="slp__timeline-chip">
          {model.runningCount} running
        </span>
        <span className="slp__timeline-chip">
          {model.decisionCount} scheduler decisions
        </span>
        <span className="slp__timeline-chip">
          {model.holCount} head-of-line blocks
        </span>
      </div>

      <div className="slp__timeline-legend">
        <span className="slp__timeline-legend-item">
          <span className="slp__timeline-swatch slp__timeline-swatch--running" />
          Running
        </span>
        <span className="slp__timeline-legend-item">
          <span className="slp__timeline-swatch slp__timeline-swatch--completed" />
          Completed
        </span>
        <span className="slp__timeline-legend-item">
          <span className="slp__timeline-swatch slp__timeline-swatch--failed" />
          Failed
        </span>
        <span className="slp__timeline-legend-item">
          <span className="slp__timeline-marker-icon slp__timeline-marker-icon--scheduler" />
          Scheduler
        </span>
        <span className="slp__timeline-legend-item">
          <span className="slp__timeline-marker-icon slp__timeline-marker-icon--hol" />
          Head-of-line
        </span>
      </div>

      <div className="slp__timeline-axis-row">
        <div className="slp__timeline-axis-label">Node</div>
        <div className="slp__timeline-axis">
          {tickMarks.map((tick) => (
            <span
              key={tick.pct}
              className="slp__timeline-tick-label"
              style={{ left: `${tick.pct}%` }}
            >
              {tick.label}
            </span>
          ))}
        </div>
      </div>

      <div className="slp__timeline-rows">
        {model.lanes.map((lane) => {
          const badgeColor = STATUS_COLORS[lane.status] ?? FALLBACK_STATUS_COLOR;
          return (
            <div key={lane.nodeId} className="slp__timeline-row">
              <button
                type="button"
                className="slp__timeline-node"
                onClick={() => onNodeClick(lane.nodeId)}
              >
                <span className="slp__timeline-node-name">{lane.name}</span>
                <span className="slp__timeline-node-id">#{lane.nodeId}</span>
                <span
                  className="slp__timeline-node-status"
                  style={statusBadgeStyle(badgeColor)}
                >
                  {lane.status}
                </span>
              </button>

              <div className="slp__timeline-lane">
                {tickMarks.map((tick) => (
                  <span
                    key={tick.pct}
                    className="slp__timeline-gridline"
                    style={{ left: `${tick.pct}%` }}
                    aria-hidden="true"
                  />
                ))}

                {lane.segments.map((segment, index) => {
                  const startPct = pctOf(segment.startMs, model.rangeStartMs, model.rangeMs);
                  const endPct = pctOf(segment.endMs, model.rangeStartMs, model.rangeMs);
                  const widthPct = Math.max(endPct - startPct, MIN_SEGMENT_WIDTH_PCT);
                  const durationMs = Math.max(segment.endMs - segment.startMs, 0);
                  const title = [
                    `${lane.name} (${segment.opType})`,
                    `${segment.status} in ${formatDuration(durationMs)}`,
                    `Start ${formatPreciseTimestamp(segment.startedAt)}`,
                    segment.endedAt
                      ? `End ${formatPreciseTimestamp(segment.endedAt)}`
                      : "Still running",
                  ].join("\n");

                  return (
                    <button
                      key={`${segment.nodeId}-${segment.startedAt}-${index}`}
                      type="button"
                      className={`slp__timeline-segment slp__timeline-segment--${segment.status}`}
                      style={{ left: `${startPct}%`, width: `${widthPct}%` }}
                      title={title}
                      onClick={() => onNodeClick(segment.nodeId)}
                    >
                      <span className="slp__timeline-segment-label">
                        {segment.opType}
                      </span>
                    </button>
                  );
                })}

                {lane.markers.map((marker, index) => {
                  const markerPct = pctOf(marker.atMs, model.rangeStartMs, model.rangeMs);
                  const topOffset = 5 + (index % 2) * 12;
                  return (
                    <span
                      key={`${marker.kind}-${marker.nodeId}-${marker.atTimestamp}-${index}`}
                      className={`slp__timeline-marker slp__timeline-marker--${marker.kind === EK.SCHEDULER_DECISION.name ? "scheduler" : "hol"}`}
                      style={{ left: `${markerPct}%`, top: `${topOffset}px` }}
                      title={[
                        marker.label,
                        formatPreciseTimestamp(marker.atTimestamp),
                        marker.summary,
                      ].join("\n")}
                      aria-label={marker.summary}
                    />
                  );
                })}
              </div>
            </div>
          );
        })}
      </div>
    </div>
  );
}

function buildTimelineModel(
  events: TraceEvent[],
  liveNodeStates: Record<number, NodeLiveStatus>,
  nodeNames: Record<number, string>,
  nowMs: number,
): TimelineModel | null {
  const lanes = new Map<number, TimelineLane>();
  const activeSegments = new Map<number, TimelineSegment[]>();

  let rangeStartMs = Number.POSITIVE_INFINITY;
  let rangeEndMs = 0;
  let decisionCount = 0;
  let holCount = 0;

  const ensureLane = (nodeId: number, atMs: number): TimelineLane => {
    const existing = lanes.get(nodeId);
    if (existing) {
      existing.firstMs = Math.min(existing.firstMs, atMs);
      return existing;
    }

    const created: TimelineLane = {
      nodeId,
      name: nodeNames[nodeId] ?? `Node #${nodeId}`,
      status: liveNodeStates[nodeId] ?? "pending",
      firstMs: atMs,
      segments: [],
      markers: [],
    };
    lanes.set(nodeId, created);
    return created;
  };

  for (const event of events) {
    const atMs = timestampMs(event.timestamp);
    if (atMs == null) {
      continue;
    }

    rangeStartMs = Math.min(rangeStartMs, atMs);
    rangeEndMs = Math.max(rangeEndMs, atMs);

    if (event.kind === EK.OPERATION_START.name) {
      const nodeId = numberField(event, "node_id");
      if (nodeId == null) {
        continue;
      }
      ensureLane(nodeId, atMs);
      const segment: TimelineSegment = {
        nodeId,
        opType: String(event.op_type ?? "op"),
        startMs: atMs,
        endMs: atMs,
        startedAt: event.timestamp,
        endedAt: null,
        status: "running",
      };
      const nodeSegments = activeSegments.get(nodeId);
      if (nodeSegments) {
        nodeSegments.push(segment);
      } else {
        activeSegments.set(nodeId, [segment]);
      }
      continue;
    }

    if (event.kind === "operation_complete" || event.kind === "operation_error") {
      const nodeId = numberField(event, "node_id");
      if (nodeId == null) {
        continue;
      }

      const nodeSegments = activeSegments.get(nodeId);
      const openSegment = nodeSegments?.pop();
      const durationMs = Math.max(numberField(event, "duration_ms") ?? 0, 0);
      const startMs = openSegment?.startMs ?? Math.max(atMs - durationMs, 0);
      const lane = ensureLane(nodeId, startMs);
      lane.segments.push({
        nodeId,
        opType: String(event.op_type ?? openSegment?.opType ?? "op"),
        startMs,
        endMs: Math.max(atMs, startMs),
        startedAt: openSegment?.startedAt ?? event.timestamp,
        endedAt: event.timestamp,
        status: event.kind === "operation_complete" ? "completed" : "failed",
      });
      lane.status = liveNodeStates[nodeId] ?? lane.segments[lane.segments.length - 1].status;
      rangeStartMs = Math.min(rangeStartMs, startMs);
      rangeEndMs = Math.max(rangeEndMs, atMs);
      continue;
    }

    if (event.kind === EK.SCHEDULER_DECISION.name) {
      const nodeId = numberField(event, "node_id");
      if (nodeId == null) {
        continue;
      }
      const lane = ensureLane(nodeId, atMs);
      lane.markers.push({
        nodeId,
        atMs,
        atTimestamp: event.timestamp,
        kind: EK.SCHEDULER_DECISION.name,
        label: "Scheduler",
        summary: eventSummary(event),
      });
      decisionCount += 1;
      continue;
    }

    if (event.kind === EK.HEAD_OF_LINE_BLOCK.name) {
      const blockedNode = numberField(event, "blocked_node");
      if (blockedNode == null) {
        continue;
      }
      const lane = ensureLane(blockedNode, atMs);
      lane.markers.push({
        nodeId: blockedNode,
        atMs,
        atTimestamp: event.timestamp,
        kind: EK.HEAD_OF_LINE_BLOCK.name,
        label: "Head-of-line block",
        summary: eventSummary(event),
      });
      holCount += 1;
    }
  }

  for (const [nodeId, pendingSegments] of activeSegments) {
    const lane = ensureLane(nodeId, nowMs);
    for (const segment of pendingSegments) {
      lane.segments.push({
        ...segment,
        endMs: Math.max(nowMs, segment.startMs),
        status: "running",
      });
      rangeStartMs = Math.min(rangeStartMs, segment.startMs);
      rangeEndMs = Math.max(rangeEndMs, nowMs);
    }
  }

  const laneList = Array.from(lanes.values())
    .map((lane) => ({
      ...lane,
      name: nodeNames[lane.nodeId] ?? lane.name,
      status: liveNodeStates[lane.nodeId] ?? deriveLaneStatus(lane),
      segments: lane.segments.sort((a, b) => a.startMs - b.startMs),
      markers: lane.markers.sort((a, b) => a.atMs - b.atMs),
    }))
    .sort((a, b) => {
      if (a.firstMs !== b.firstMs) {
        return a.firstMs - b.firstMs;
      }
      return a.nodeId - b.nodeId;
    });

  if (laneList.length === 0) {
    return null;
  }

  if (!Number.isFinite(rangeStartMs)) {
    rangeStartMs = Math.max(rangeEndMs, nowMs);
  }
  rangeEndMs = Math.max(rangeEndMs, rangeStartMs);
  const rangeMs = Math.max(rangeEndMs - rangeStartMs, 1);
  const runningCount = laneList.filter((lane) => lane.status === "running").length;

  return {
    lanes: laneList,
    rangeStartMs,
    rangeEndMs,
    rangeMs,
    runningCount,
    decisionCount,
    holCount,
  };
}

function deriveLaneStatus(lane: TimelineLane): NodeLiveStatus {
  const lastSegment = lane.segments[lane.segments.length - 1];
  return lastSegment?.status ?? "pending";
}

function pctOf(valueMs: number, startMs: number, rangeMs: number): number {
  return Math.min(Math.max(((valueMs - startMs) / rangeMs) * 100, 0), 100);
}

function timestampMs(timestamp: string): number | null {
  const ms = new Date(timestamp).getTime();
  return Number.isNaN(ms) ? null : ms;
}

function numberField(event: TraceEvent, key: string): number | null {
  const value = event[key];
  return typeof value === "number" && Number.isFinite(value) ? value : null;
}
