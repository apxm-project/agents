import { useAppStore } from "@/store/app-store";

export function GraphAnalysisStrip() {
  const analysis = useAppStore((s) => s.graphAnalysis);
  if (!analysis) return null;

  const speedup = analysis.max_parallelism > 0
    ? (analysis.critical_path_length / analysis.max_parallelism).toFixed(2)
    : "N/A";

  return (
    <div className="analysis-strip">
      <span>Nodes: {analysis.total_nodes}</span>
      <span>Edges: {analysis.total_edges}</span>
      <span>Critical Path: {analysis.critical_path_length}</span>
      <span>Max Parallelism: {analysis.max_parallelism}</span>
      <span>Speedup: {speedup}x</span>
    </div>
  );
}
