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

export type StartupInfo = {
  initial_file: string | null;
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

export type PassInfo = {
  name: string;
  category: string;
  summary: string;
  description: string;
};

export type FileTreeNode = {
  name: string;
  path: string;
  is_dir: boolean;
  children?: FileTreeNode[];
  apxm_meta?: { name: string | null; node_count: number | null };
};

export type FileTreeResponse = {
  root: string;
  cwd: string;
  tree: FileTreeNode[];
};
