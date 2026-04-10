import { useCallback } from "react";
import { useSSE } from "./use-sse";
import { useAppStore } from "@/store/app-store";
import type { TraceEvent } from "@/types/events";

export function useLiveSession() {
  const liveSessionPath = useAppStore((s) => s.liveSessionPath);
  const addLiveEvent = useAppStore((s) => s.addLiveEvent);
  const setLiveNodeStatus = useAppStore((s) => s.setLiveNodeStatus);
  const setLiveManifest = useAppStore((s) => s.setLiveManifest);
  const setLiveActive = useAppStore((s) => s.setLiveActive);

  const url = liveSessionPath
    ? `/api/live/session?path=${encodeURIComponent(liveSessionPath)}`
    : null;

  const handleEvent = useCallback(
    (eventType: string, data: string) => {
      try {
        const parsed = JSON.parse(data);
        if (eventType === "trace") {
          addLiveEvent(parsed as TraceEvent);
          const trace = parsed as TraceEvent;
          if (trace.kind === "operation_start" && trace.node_id != null) {
            setLiveNodeStatus(trace.node_id, "running");
          } else if (trace.kind === "operation_complete" && trace.node_id != null) {
            setLiveNodeStatus(trace.node_id, "completed");
          } else if (trace.kind === "operation_error" && trace.node_id != null) {
            setLiveNodeStatus(trace.node_id, "failed");
          }
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
    [addLiveEvent, setLiveNodeStatus, setLiveManifest, setLiveActive],
  );

  useSSE({ url, onEvent: handleEvent });
}
