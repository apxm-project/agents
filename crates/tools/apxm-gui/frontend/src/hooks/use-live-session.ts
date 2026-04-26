import { useCallback } from "react";
import { useSSE } from "./use-sse";
import { useAppStore } from "@/store/app-store";
import * as EK from "@/lib/event-kinds";
import type { TraceEvent } from "@/types/events";

/**
 * Normalize a raw trace event from the backend (which uses the Rust
 * ApxmEvent envelope: { meta: { timestamp, ... }, payload: { kind, ... } })
 * into the flat TraceEvent shape the frontend expects.
 */
function normalizeTraceEvent(raw: Record<string, unknown>): TraceEvent {
  // Already flat format from a pre-normalized source.
  if (typeof raw.kind === "string" && typeof raw.timestamp === "string") {
    return raw as TraceEvent;
  }

  const meta = raw.meta as Record<string, unknown> | undefined;
  const payload = raw.payload as Record<string, unknown> | undefined;

  if (!payload || typeof payload.kind !== "string") {
    return { timestamp: String(meta?.timestamp ?? ""), kind: "unknown", ...raw };
  }

  const kind = payload.kind as string;
  const timestamp = String(meta?.timestamp ?? "");

  // Map Rust operation_end (with success flag) to frontend operation_complete/error
  if (kind === EK.OPERATION_END.name) {
    const success = payload.success as boolean;
    return {
      timestamp,
      kind: success ? "operation_complete" : "operation_error",
      node_id: payload.node_id as number | undefined,
      op_type: payload.op_type as string | undefined,
      duration_ms: payload.duration_ms as number | undefined,
      ...(!success && payload.error ? { error: payload.error } : {}),
    };
  }

  // Flatten all other events: lift payload fields to top level
  const { kind: _k, ...payloadRest } = payload;
  return {
    timestamp,
    kind,
    ...payloadRest,
  } as TraceEvent;
}

export function useLiveSession(sessionPathOverride?: string | null) {
  const storeSessionPath = useAppStore((s) => s.liveSessionPath);
  const addLiveEvent = useAppStore((s) => s.addLiveEvent);
  const setLiveNodeStatus = useAppStore((s) => s.setLiveNodeStatus);
  const setLiveManifest = useAppStore((s) => s.setLiveManifest);
  const setLiveActive = useAppStore((s) => s.setLiveActive);
  const setLiveProgress = useAppStore((s) => s.setLiveProgress);

  const liveSessionPath = sessionPathOverride !== undefined ? sessionPathOverride : storeSessionPath;

  const url = liveSessionPath
    ? `/api/live/session?path=${encodeURIComponent(liveSessionPath)}`
    : null;

  const handleEvent = useCallback(
    (eventType: string, data: string) => {
      try {
        const parsed = JSON.parse(data);
        if (eventType === "trace") {
          const trace = normalizeTraceEvent(parsed as Record<string, unknown>);
          addLiveEvent(trace);
          if (trace.kind === EK.OPERATION_START.name && trace.node_id != null) {
            setLiveNodeStatus(trace.node_id, "running");
          } else if (trace.kind === "operation_complete" && trace.node_id != null) {
            setLiveNodeStatus(trace.node_id, "completed");
          } else if (trace.kind === "operation_error" && trace.node_id != null) {
            setLiveNodeStatus(trace.node_id, "failed");
          }
        } else if (eventType === "progress") {
          const progress = parsed as Record<string, unknown>;
          setLiveProgress(progress);
          const running = progress.running_nodes as number[] | undefined;
          const completed = progress.completed_nodes as number[] | undefined;
          const failed = progress.failed_nodes as number[] | undefined;
          if (running) for (const id of running) setLiveNodeStatus(id, "running");
          if (completed) for (const id of completed) setLiveNodeStatus(id, "completed");
          if (failed) for (const id of failed) setLiveNodeStatus(id, "failed");
        } else if (eventType === "status") {
          setLiveManifest(parsed as Record<string, unknown>);
          const status = (parsed as Record<string, unknown>).status;
          if (status === "completed" || status === "failed") {
            setLiveActive(false);
          }
        }
      } catch {
        // ignore parse errors
      }
    },
    [addLiveEvent, setLiveNodeStatus, setLiveManifest, setLiveActive, setLiveProgress],
  );

  useSSE({ url, onEvent: handleEvent });
}
