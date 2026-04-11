import { useCallback, useMemo, useState } from "react";
import { useAppStore } from "@/store/app-store";
import { fetchGraph } from "@/api";
import { compileWorkflow } from "@/api/compile";
import { CompilePanel } from "@/components/studio/compile-panel";
import { highlightLine } from "@/lib/syntax-highlight";
import { ViewMode, LoadKey } from "@/lib/constants";

export function SourceView() {
  const sourceContent = useAppStore((s) => s.sourceContent);
  const sourcePath = useAppStore((s) => s.sourcePath);
  const selectedWorkflow = useAppStore((s) => s.selectedWorkflow);
  const setViewMode = useAppStore((s) => s.setViewMode);
  const setGraphData = useAppStore((s) => s.setGraphData);
  const setLoading = useAppStore((s) => s.setLoading);
  const setError = useAppStore((s) => s.setError);
  const setCompileResult = useAppStore((s) => s.setCompileResult);
  const setExecuteDialogOpen = useAppStore((s) => s.setExecuteDialogOpen);
  const compileResult = useAppStore((s) => s.compileResult);
  const loading = useAppStore((s) => s.loading[LoadKey.SOURCE]);

  const [compiling, setCompiling] = useState(false);

  const fileName = useMemo(() => {
    if (!sourcePath) return null;
    return sourcePath.split("/").pop() ?? sourcePath;
  }, [sourcePath]);

  const language = useMemo(() => {
    if (!sourcePath) return "text";
    if (sourcePath.endsWith(".py")) return "python";
    if (sourcePath.endsWith(".air")) return "mlir";
    if (sourcePath.endsWith(".json")) return "json";
    return "text";
  }, [sourcePath]);

  const lineCount = useMemo(
    () => sourceContent?.split("\n").length ?? 0,
    [sourceContent],
  );

  const onViewGraph = useCallback(async () => {
    if (!selectedWorkflow) return;
    setLoading(LoadKey.GRAPH, true);
    try {
      const graph = await fetchGraph(selectedWorkflow.path);
      setGraphData(graph, selectedWorkflow.path);
      setViewMode(ViewMode.GRAPH);
      setError(LoadKey.GRAPH, null);
    } catch (e: unknown) {
      setError(LoadKey.GRAPH, e instanceof Error ? e.message : String(e));
    } finally {
      setLoading(LoadKey.GRAPH, false);
    }
  }, [selectedWorkflow, setGraphData, setViewMode, setLoading, setError]);

  const onCompile = useCallback(async () => {
    if (!selectedWorkflow || compiling) return;
    setCompiling(true);
    try {
      const result = await compileWorkflow(selectedWorkflow.path);
      setCompileResult(result);
      setError(LoadKey.COMPILE, null);
    } catch (e: unknown) {
      setError(LoadKey.COMPILE, e instanceof Error ? e.message : String(e));
    } finally {
      setCompiling(false);
    }
  }, [selectedWorkflow, compiling, setCompileResult, setError]);

  const onExecute = useCallback(() => {
    setExecuteDialogOpen(true);
  }, [setExecuteDialogOpen]);

  if (loading) {
    return (
      <div className="viewer-status">
        <div className="viewer-status__spinner" />
        <p className="viewer-status__hint">Loading source...</p>
      </div>
    );
  }

  if (!sourceContent) {
    return (
      <div className="viewer-status">
        <h2 className="viewer-status__title">APXM Agent Studio</h2>
        <p className="viewer-status__hint">
          Select a workflow from the sidebar to preview its source code,
          compile it, visualize the graph, and execute it live.
        </p>
      </div>
    );
  }

  return (
    <div className="source-view">
      <div className="source-view__toolbar">
        <div className="source-view__info">
          <span className="source-view__filename">{fileName}</span>
          <span className="source-view__meta">
            {language} · {lineCount} lines
            {selectedWorkflow?.node_count != null && (
              <> · {selectedWorkflow.node_count} nodes</>
            )}
          </span>
        </div>
        <div className="source-view__actions">
          <button
            type="button"
            className={`ghost-button${compiling ? " ghost-button--loading" : ""}`}
            onClick={onCompile}
            disabled={compiling}
          >
            {compiling ? "Compiling..." : "Compile"}
          </button>
          <button
            type="button"
            className="ghost-button ghost-button--execute"
            onClick={onExecute}
          >
            Execute
          </button>
          <button
            type="button"
            className="ghost-button"
            onClick={onViewGraph}
          >
            View Graph →
          </button>
        </div>
      </div>
      {compileResult && <CompilePanel />}
      <div className="source-view__content">
        <pre className="source-view__code">
          <table className="source-view__lines">
            <tbody>
              {sourceContent.split("\n").map((line, i) => (
                <tr key={i} className="source-view__line">
                  <td className="source-view__line-number">{i + 1}</td>
                  <td className="source-view__line-content">
                    {highlightLine(line, language)}
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </pre>
      </div>
    </div>
  );
}
