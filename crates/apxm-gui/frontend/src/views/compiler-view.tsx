import { useState } from "react";
import { useAppStore } from "@/store/app-store";
import { fetchOptimized } from "@/api/graph";
import { getErrorMessage } from "@/lib/format";

export function CompilerView() {
  const graphPath = useAppStore((s) => s.graphPath);
  const optimizedResult = useAppStore((s) => s.optimizedResult);
  const setOptimizedResult = useAppStore((s) => s.setOptimizedResult);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);

  async function handleOptimize() {
    if (!graphPath) return;
    setLoading(true);
    setError(null);
    try {
      const result = await fetchOptimized(graphPath);
      setOptimizedResult(result);
    } catch (e) {
      setError(getErrorMessage(e));
    }
    setLoading(false);
  }

  return (
    <div className="view-panel">
      <div className="view-panel__header">
        <h2>Compiler</h2>
        {graphPath ? (
          <button type="button" className="ghost-button" onClick={handleOptimize} disabled={loading}>
            {loading ? "Optimizing..." : "Run Optimization"}
          </button>
        ) : null}
      </div>
      {error ? <div className="error-banner">{error}</div> : null}
      {!graphPath ? (
        <div className="view-panel__empty">Load a graph first to use the compiler view.</div>
      ) : null}
      {optimizedResult ? (
        <div className="compiler-results">
          {optimizedResult.note ? (
            <div className="compiler-results__note">{optimizedResult.note}</div>
          ) : null}
          <div className="compiler-results__passes">
            <strong>Passes applied:</strong> {optimizedResult.passes_applied.length > 0 ? optimizedResult.passes_applied.join(", ") : "none"}
          </div>
          {optimizedResult.diff.length > 0 ? (
            <div className="compiler-results__diff">
              <strong>Changes ({optimizedResult.diff.length}):</strong>
              <ul>
                {optimizedResult.diff.map((d) => (
                  <li key={d.node_id}>
                    Node #{d.node_id} ({d.node_name}): {Object.keys(d.added_attributes).join(", ")}
                  </li>
                ))}
              </ul>
            </div>
          ) : (
            <div className="compiler-results__diff">No changes detected.</div>
          )}
        </div>
      ) : null}
    </div>
  );
}
