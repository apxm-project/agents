import { useState } from "react";
import { useAppStore } from "@/store/app-store";
import { decompileArtifact } from "@/api/compile";

export function CompilePanel() {
  const result = useAppStore((s) => s.compileResult);
  const setCompileResult = useAppStore((s) => s.setCompileResult);
  const setExecuteDialogOpen = useAppStore((s) => s.setExecuteDialogOpen);
  const [decompiling, setDecompiling] = useState(false);
  const [decompiled, setDecompiled] = useState<Record<string, unknown> | null>(null);
  const [showDiagnostics, setShowDiagnostics] = useState(false);

  if (!result) return null;

  const nodesDelta =
    result.node_count_before != null && result.node_count_after != null
      ? result.node_count_after - result.node_count_before
      : null;

  async function handleDecompile() {
    if (!result?.artifact_path || decompiling) return;
    setDecompiling(true);
    try {
      const res = await decompileArtifact(result.artifact_path);
      if (res.success && res.graph) setDecompiled(res.graph);
    } catch { /* ignore */ }
    setDecompiling(false);
  }

  return (
    <div className="compile-panel">
      <div className="compile-panel__header">
        <span className="compile-panel__title">
          {result.success ? "Compilation Succeeded" : "Compilation Failed"}
        </span>
        <div className="compile-panel__header-actions">
          {result.success && (
            <button
              type="button"
              className="ghost-button ghost-button--sm ghost-button--execute"
              onClick={() => setExecuteDialogOpen(true)}
            >
              Execute →
            </button>
          )}
          <button
            type="button"
            className="compile-panel__close"
            onClick={() => { setCompileResult(null); setDecompiled(null); }}
          >
            &times;
          </button>
        </div>
      </div>

      <div className="compile-panel__metrics">
        <div className="compile-panel__metric">
          <span className="compile-panel__metric-value">{result.duration_ms}ms</span>
          <span className="compile-panel__metric-label">Duration</span>
        </div>
        {result.node_count_before != null && (
          <div className="compile-panel__metric">
            <span className="compile-panel__metric-value">{result.node_count_before}</span>
            <span className="compile-panel__metric-label">Nodes (before)</span>
          </div>
        )}
        {result.node_count_after != null && (
          <div className="compile-panel__metric">
            <span className="compile-panel__metric-value">{result.node_count_after}</span>
            <span className="compile-panel__metric-label">Nodes (after)</span>
          </div>
        )}
        {nodesDelta != null && (
          <div className="compile-panel__metric">
            <span
              className="compile-panel__metric-value"
              style={{ color: nodesDelta < 0 ? "var(--success)" : nodesDelta > 0 ? "var(--warning)" : "var(--ink-faint)" }}
            >
              {nodesDelta > 0 ? `+${nodesDelta}` : nodesDelta}
            </span>
            <span className="compile-panel__metric-label">Delta</span>
          </div>
        )}
      </div>

      {result.passes.length > 0 && (
        <div className="compile-panel__passes">
          <span className="compile-panel__section-title">Pass Pipeline (O{result.passes.length > 10 ? "2+" : "1"})</span>
          <div className="compile-panel__pass-list">
            {result.passes.map((pass, i) => (
              <div key={i} className="compile-panel__pass">
                <span className="compile-panel__pass-index">{i + 1}</span>
                <span className="compile-panel__pass-name">{pass}</span>
              </div>
            ))}
          </div>
        </div>
      )}

      {result.artifact_path && (
        <div className="compile-panel__artifact">
          <span className="compile-panel__section-title">Artifact</span>
          <div className="compile-panel__artifact-row">
            <code className="compile-panel__artifact-path">
              {result.artifact_path.split("/").pop()}
            </code>
            <button
              type="button"
              className={`ghost-button ghost-button--sm${decompiling ? " ghost-button--loading" : ""}`}
              onClick={handleDecompile}
              disabled={decompiling}
            >
              {decompiling ? "..." : "Decompile"}
            </button>
          </div>
        </div>
      )}

      {result.diagnostics && (
        <div className="compile-panel__diagnostics">
          <button
            type="button"
            className="compile-panel__section-title compile-panel__section-title--toggle"
            onClick={() => setShowDiagnostics(!showDiagnostics)}
          >
            Diagnostics {showDiagnostics ? "▾" : "▸"}
          </button>
          {showDiagnostics && (
            <pre className="compile-panel__diag-json">{JSON.stringify(result.diagnostics, null, 2)}</pre>
          )}
        </div>
      )}

      {decompiled && (
        <div className="compile-panel__decompiled">
          <span className="compile-panel__section-title">Decompiled Graph</span>
          <pre className="compile-panel__decompile-json">{JSON.stringify(decompiled, null, 2)}</pre>
        </div>
      )}

      {!result.success && result.stderr && (
        <div className="compile-panel__error">
          <span className="compile-panel__section-title">Error Output</span>
          <pre className="compile-panel__stderr">{result.stderr}</pre>
        </div>
      )}
    </div>
  );
}
