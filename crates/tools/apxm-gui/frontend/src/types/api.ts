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
  source_type: "py" | "air" | "apxm";
  description?: string;
  parameters?: { name: string; type_name: string }[];
  air_path?: string;
};

export type BackendHealth = {
  name: string;
  endpoint: string;
  protocol: string;
  model_count: number;
  status: "healthy" | "degraded" | "unreachable" | "unknown";
};

export type ModelDetail = {
  id: string;
  aliases: string[];
  context_window: number;
  supports_vision: boolean;
  supports_functions: boolean;
  tags: string[];
};

export type BackendDetail = {
  name: string;
  endpoint: string;
  protocol: string;
  backend_type: string;
  model_count: number;
  models: ModelDetail[];
  status: "healthy" | "unreachable" | "unknown";
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

export type CompileResult = {
  success: boolean;
  artifact_path: string | null;
  passes: string[];
  duration_ms: number;
  stdout: string;
  stderr: string;
  node_count_before: number | null;
  node_count_after: number | null;
  diagnostics: Record<string, unknown> | null;
};

export type ValidateResult = {
  valid: boolean;
  stdout: string;
  stderr: string;
  details: Record<string, unknown> | null;
};

export type DecompileResult = {
  success: boolean;
  graph: Record<string, unknown> | null;
  stderr: string;
};

export type ExplainResult = {
  success: boolean;
  explanation: string;
  stderr: string;
};

export type AgentProfile = {
  name: string;
  description: string;
  skills: string[];
  category: string;
};

export type ExecuteResult = {
  session_path: string;
  execution_id: string;
};

export type SkillInfo = {
  name: string;
  description: string;
  user_invocable: boolean;
  heading: string | null;
  body_length: number;
  section_count: number;
  category: string;
};

export type SkillDetail = SkillInfo & {
  content: string;
};

export type FileTreeNode = {
  name: string;
  path: string;
  is_dir: boolean;
  children?: FileTreeNode[];
  graph_meta?: { name: string | null; node_count: number | null };
};

export type FileTreeResponse = {
  root: string;
  cwd: string;
  tree: FileTreeNode[];
};
