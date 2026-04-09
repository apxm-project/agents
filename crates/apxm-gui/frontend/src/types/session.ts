import type { TraceEvent } from "./events";

export type SessionManifest = {
  session_id?: string;
  graph_name?: string;
  status: string;
  timestamp?: string;
  duration_ms?: number;
  node_count?: number;
  [key: string]: unknown;
};

export type SessionData = {
  manifest: SessionManifest | null;
  trace: TraceEvent[];
  results: Record<string, unknown> | null;
  metrics: Record<string, unknown> | null;
  node_statuses: Record<string, unknown> | null;
};

export type SessionInfo = {
  id: string;
  graph_name: string | null;
  status: string;
  started_at: string;
  duration_ms?: number;
};
