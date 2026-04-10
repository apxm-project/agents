export type CategoryStyle = {
  color: string;
  bg: string;
  gradient: string;
};

const CATEGORY_STYLES: Record<string, CategoryStyle> = {
  reasoning:       { color: "#3ea6c6", bg: "rgba(62, 166, 198, 0.15)",  gradient: "rgba(56, 144, 194, 0.12)" },
  tools:           { color: "#d49b45", bg: "rgba(212, 155, 69, 0.16)",  gradient: "rgba(193, 148, 68, 0.10)" },
  memory:          { color: "#9f84e0", bg: "rgba(159, 132, 224, 0.14)", gradient: "rgba(142, 118, 204, 0.10)" },
  synchronization: { color: "#58bb81", bg: "rgba(88, 187, 129, 0.14)",  gradient: "rgba(98, 176, 157, 0.08)" },
  control_flow:    { color: "#66758b", bg: "rgba(102, 117, 139, 0.14)", gradient: "rgba(102, 117, 139, 0.08)" },
  error_handling:  { color: "#d86b6b", bg: "rgba(216, 107, 107, 0.14)", gradient: "rgba(196, 97, 97, 0.08)" },
  communication:   { color: "#5c80d6", bg: "rgba(92, 128, 214, 0.16)",  gradient: "rgba(92, 128, 214, 0.10)" },
  coordination:    { color: "#e07c9f", bg: "rgba(224, 124, 159, 0.14)", gradient: "rgba(204, 108, 143, 0.10)" },
  identity:        { color: "#8d99a7", bg: "rgba(141, 153, 167, 0.12)", gradient: "rgba(141, 153, 167, 0.06)" },
  internal:        { color: "#66758b", bg: "rgba(102, 117, 139, 0.10)", gradient: "rgba(102, 117, 139, 0.06)" },
};

const DEFAULT_STYLE: CategoryStyle = {
  color: "#8d99a7",
  bg: "rgba(141, 153, 167, 0.12)",
  gradient: "rgba(141, 153, 167, 0.06)",
};

export function getCategoryStyle(category: string): CategoryStyle {
  return CATEGORY_STYLES[category.toLowerCase()] ?? DEFAULT_STYLE;
}

export type EdgeStyle = {
  stroke: string;
  strokeDasharray?: string;
  markerColor: string;
};

const EDGE_STYLES: Record<string, EdgeStyle> = {
  Data:    { stroke: "#3ea6c6", markerColor: "#3ea6c6" },
  Control: { stroke: "#d86b6b", strokeDasharray: "8 5", markerColor: "#d86b6b" },
  Effect:  { stroke: "#d49b45", strokeDasharray: "3 3", markerColor: "#d49b45" },
};

const DEFAULT_EDGE_STYLE: EdgeStyle = {
  stroke: "rgba(133, 146, 166, 0.24)",
  markerColor: "rgba(133, 146, 166, 0.24)",
};

export function getEdgeStyle(dependency: string): EdgeStyle {
  return EDGE_STYLES[dependency] ?? DEFAULT_EDGE_STYLE;
}
