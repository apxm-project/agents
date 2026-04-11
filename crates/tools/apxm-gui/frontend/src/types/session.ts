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

export type NodeStatusEntry = {
  node_id: number;
  status: string;
  retries: number;
  last_error: string | null;
  started_at_ms?: number;
  finished_at_ms?: number;
  duration_ms?: number;
};

export type SessionData = {
  manifest: SessionManifest | null;
  trace: TraceEvent[];
  results: Record<string, unknown> | null;
  metrics: Record<string, unknown> | null;
  node_statuses: NodeStatusEntry[] | Record<string, unknown> | null;
  node_names: Record<string, string> | null;
  source_path: string | null;
};

export type SessionInfo = {
  id: string;
  graph_name: string | null;
  status: string;
  started_at: string;
  duration_ms?: number | null;
  node_count?: number | null;
  mtime_epoch?: number | null;
};
