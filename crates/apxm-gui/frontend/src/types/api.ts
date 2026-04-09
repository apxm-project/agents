export type GraphAnalysis = {
  total_nodes: number;
  total_edges: number;
  entry_nodes: number[];
  exit_nodes: number[];
  critical_path_length: number;
  critical_path: number[];
  max_parallelism: number;
  op_histogram: Record<string, number>;
};

export type OptimizedResult = {
  original: import("./graph").ApxmGraph;
  optimized: import("./graph").ApxmGraph;
  passes_applied: string[];
  diff: Array<{
    node_id: number;
    node_name: string;
    added_attributes: Record<string, unknown>;
  }>;
  note?: string;
};

export type StartupInfo = {
  initial_file: string | null;
};

export type ExampleInfo = {
  path: string;
  relative_path: string;
  name: string;
  category: string;
  node_count: number | null;
};

export type WorkflowInfo = {
  path: string;
  relative_path: string;
  name: string;
  category: string;
  node_count: number | null;
};

export type BackendHealth = {
  name: string;
  endpoint: string;
  protocol: string;
  model_count: number;
  status: "healthy" | "degraded" | "unreachable" | "unknown";
};

export type HealthStatus = {
  backends: BackendHealth[];
  total_models: number;
  total_agents: number;
  total_tools: number;
  config_source: "project" | "global";
};
