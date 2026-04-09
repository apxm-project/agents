export type ApxmGraph = {
  name: string;
  nodes: GraphNode[];
  edges: GraphEdge[];
  parameters: GraphParameter[];
  metadata: Record<string, unknown>;
};

export type GraphNode = {
  id: number;
  name: string;
  op: string;
  attributes: Record<string, unknown>;
};

export type GraphEdge = {
  from: number;
  to: number;
  dependency: DependencyType;
};

export type DependencyType = "Data" | "Control" | "Effect";

export type GraphParameter = {
  name: string;
  type_name: string;
};

export type ViewerPoint = { x: number; y: number };

export type GraphLayout = {
  nodePositions: Record<string, ViewerPoint>;
  edgeRoutes: Record<string, { points: ViewerPoint[] }>;
};

export type AisNodeData = {
  nodeId: number;
  name: string;
  op: string;
  category: string;
  latency: string;
  description: string;
  producesOutput: boolean;
  attributes: Record<string, unknown>;
  isEntry: boolean;
  isTerminal: boolean;
  incomingCount: number;
  outgoingCount: number;
  selected: boolean;
  liveStatus?: import("./events").NodeLiveStatus;
};

export type DepEdgeData = {
  dependency: DependencyType;
  points?: ViewerPoint[];
};
