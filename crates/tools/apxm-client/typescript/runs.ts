// Server run wire shapes (apxm-server runs.rs). Consumed by Studio runs/observability.

export type NodeStatus = "pending" | "running" | "succeeded" | "failed";

export interface RunSummary {
  execution_id: string;
  skill_id: string;
  skill_version: string;
  status: string;
  started_at_ms: number;
  completed_at_ms?: number;
  session_id: string;
  root_agent?: string;
  correlation_id?: string | null;
  totals: { events: number; nodes: number; edges: number; tool_calls: number };
}

export interface RunListResponse {
  object: string;
  data: RunSummary[];
}

export interface RunGraphNode {
  id: number;
  kind: "agent" | "tool" | "llm" | "op";
  op_type: string;
  agent_code?: string;
  tool_name?: string;
  status: NodeStatus;
  started_at_ms?: number;
  completed_at_ms?: number;
  duration_ms?: number;
  layer?: number;
  context?: unknown;
}

export interface RunGraphEdge {
  from: number;
  to: number;
  kind: string;
}

export interface RunGraphWire {
  graph_schema_version: number;
  execution_id: string;
  nodes: RunGraphNode[];
  edges: RunGraphEdge[];
}
