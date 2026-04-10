export type TraceEvent = {
  timestamp: string;
  kind: string;
  node_id?: number;
  [key: string]: unknown;
};

export type EventPayload =
  | { kind: "token"; text: string; node_id: number }
  | { kind: "operation_start"; node_id: number; op_type: string }
  | { kind: "operation_complete"; node_id: number; op_type: string; duration_ms: number }
  | { kind: "operation_error"; node_id: number; error: string }
  | { kind: "session_start"; session_id: string }
  | { kind: "session_complete"; session_id: string; duration_ms: number }
  | { kind: "session_error"; session_id: string; error: string }
  | { kind: "scheduler_decision"; node_id: number; action: string }
  | { kind: "memory_read"; node_id: number; tier: string; key: string }
  | { kind: "memory_write"; node_id: number; tier: string; key: string }
  | { kind: "spawn_agent"; node_id: number; agent_type: string }
  | { kind: "agent_complete"; node_id: number; agent_type: string }
  | { kind: "checkpoint_created"; node_id: number; checkpoint_id: string }
  | { kind: "checkpoint_restored"; checkpoint_id: string }
  | { kind: "capability_invoked"; node_id: number; capability: string }
  | { kind: "retry"; node_id: number; attempt: number; reason: string }
  | { kind: string; [key: string]: unknown };

export type NodeLiveStatus = "pending" | "running" | "completed" | "failed";
