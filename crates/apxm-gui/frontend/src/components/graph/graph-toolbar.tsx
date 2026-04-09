import { useAppStore } from "@/store/app-store";
import { fetchGraphAnalysis } from "@/api/graph";
import { getErrorMessage } from "@/lib/format";

export function GraphToolbar() {
  const graphData = useAppStore((s) => s.graphData);
  const graphPath = useAppStore((s) => s.graphPath);
  const layoutDirection = useAppStore((s) => s.layoutDirection);
  const setLayoutDirection = useAppStore((s) => s.setLayoutDirection);
  const setGraphData = useAppStore((s) => s.setGraphData);
  const setGraphAnalysis = useAppStore((s) => s.setGraphAnalysis);
  const setError = useAppStore((s) => s.setError);

  function handleToggleDirection() {
    setLayoutDirection(layoutDirection === "DOWN" ? "RIGHT" : "DOWN");
  }

  async function handleOpenFile() {
    const input = document.createElement("input");
    input.type = "file";
    input.accept = ".apxm,.json";
    input.onchange = async () => {
      const file = input.files?.[0];
      if (!file) return;
      try {
        const text = await file.text();
        const graph = JSON.parse(text);
        setGraphData(graph, file.name);
        setError("graph", null);
      } catch (e) {
        setError("graph", getErrorMessage(e));
      }
    };
    input.click();
  }

  async function handleAnalyze() {
    if (!graphPath) return;
    try {
      const analysis = await fetchGraphAnalysis(graphPath);
      setGraphAnalysis(analysis);
    } catch (e) {
      setError("analysis", getErrorMessage(e));
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
      <div className="graph-toolbar__actions">
        <button type="button" className="ghost-button" onClick={handleOpenFile}>Open File</button>
        <button type="button" className="ghost-button" onClick={handleToggleDirection}>
          {layoutDirection === "DOWN" ? "\u2195 Vertical" : "\u2194 Horizontal"}
        </button>
        {graphPath ? (
          <button type="button" className="ghost-button" onClick={handleAnalyze}>Analyze</button>
        ) : null}
      </div>
    </div>
  );
}
