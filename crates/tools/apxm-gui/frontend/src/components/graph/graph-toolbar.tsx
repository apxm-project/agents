import { useState } from "react";
import { useAppStore } from "@/store/app-store";
import { fetchGraphAnalysis } from "@/api/graph";
import { getErrorMessage } from "@/lib/format";

export function GraphToolbar({ onToggleAnalysis }: { onToggleAnalysis?: () => void } = {}) {
  const graphData = useAppStore((s) => s.graphData);
  const graphPath = useAppStore((s) => s.graphPath);
  const layoutDirection = useAppStore((s) => s.layoutDirection);
  const setLayoutDirection = useAppStore((s) => s.setLayoutDirection);
  const setGraphAnalysis = useAppStore((s) => s.setGraphAnalysis);
  const setError = useAppStore((s) => s.setError);
  const searchQuery = useAppStore((s) => s.searchQuery);
  const setSearchQuery = useAppStore((s) => s.setSearchQuery);
  const toggleInspector = useAppStore((s) => s.toggleInspector);

  function handleToggleDirection() {
    setLayoutDirection(layoutDirection === "DOWN" ? "RIGHT" : "DOWN");
  }

  const [analyzing, setAnalyzing] = useState(false);

  async function handleAnalyze() {
    if (!graphPath || analyzing) return;
    setAnalyzing(true);
    try {
      const analysis = await fetchGraphAnalysis(graphPath);
      setGraphAnalysis(analysis);
    } catch (e) {
      setError("analysis", getErrorMessage(e));
    } finally {
      setAnalyzing(false);
    }
  }

  return (
    <div className="graph-toolbar">
      <div className="graph-toolbar__info">
        <span className="graph-toolbar__name">{graphData?.name ?? "No graph loaded"}</span>
        {graphData ? (
          <span className="graph-toolbar__stats">
            {graphData.nodes.length} node{graphData.nodes.length !== 1 ? "s" : ""} &middot;{" "}
            {graphData.edges.length} edge{graphData.edges.length !== 1 ? "s" : ""}
          </span>
        ) : null}
      </div>
      {graphData ? (
        <div className="graph-toolbar__search-wrap">
          <input
            type="text"
            className="graph-toolbar__search"
            placeholder="Search nodes..."
            value={searchQuery}
            onChange={(e) => setSearchQuery(e.target.value)}
          />
        </div>
      ) : null}
      <div className="graph-toolbar__actions">
        <button type="button" className="ghost-button" onClick={handleToggleDirection}>
          {layoutDirection === "DOWN" ? "\u2195 Vertical" : "\u2194 Horizontal"}
        </button>
        {graphPath ? (
          <button type="button" className="ghost-button" onClick={handleAnalyze} disabled={analyzing}>
            {analyzing ? "Analyzing..." : "Analyze"}
          </button>
        ) : null}
        {onToggleAnalysis && graphPath ? (
          <button type="button" className="ghost-button" onClick={onToggleAnalysis}>
            Analysis
          </button>
        ) : null}
        <button type="button" className="ghost-button" onClick={toggleInspector}>
          Inspector
        </button>
      </div>
    </div>
  );
}
