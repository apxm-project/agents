import { useState, useEffect } from "react";
import { useAppStore } from "@/store/app-store";
import { fetchPasses } from "@/api/graph";
import type { PassInfo } from "@/types/api";

export function GraphAnalysisPanel({ onClose }: { onClose: () => void }) {
  const analysis = useAppStore((s) => s.graphAnalysis);
  const setSelectedNodeId = useAppStore((s) => s.setSelectedNodeId);
  const [passes, setPasses] = useState<PassInfo[]>([]);
  const [passesLoaded, setPassesLoaded] = useState(false);

  useEffect(() => {
    if (!passesLoaded) {
      fetchPasses().then((p) => { setPasses(p); setPassesLoaded(true); }).catch(() => setPassesLoaded(true));
    }
  }, [passesLoaded]);

  if (!analysis) return null;

  const speedup = analysis.critical_path_length > 0
    ? (analysis.total_nodes / analysis.critical_path_length).toFixed(2)
    : "N/A";

  const histogramEntries = Object.entries(analysis.op_histogram).sort((a, b) => b[1] - a[1]);
  const maxHistCount = histogramEntries.length > 0 ? histogramEntries[0]![1] : 1;

  return (
    <div className="analysis-panel">
      <div className="analysis-panel__header">
        <strong>Graph Analysis</strong>
        <button type="button" className="analysis-panel__close" onClick={onClose}>&times;</button>
      </div>

      <div className="analysis-panel__metrics">
        <div className="analysis-panel__metric">
          <span className="analysis-panel__metric-value">{analysis.total_nodes}</span>
          <span className="analysis-panel__metric-label">Nodes</span>
        </div>
        <div className="analysis-panel__metric">
          <span className="analysis-panel__metric-value">{analysis.total_edges}</span>
          <span className="analysis-panel__metric-label">Edges</span>
        </div>
        <div className="analysis-panel__metric">
          <span className="analysis-panel__metric-value">{analysis.critical_path_length}</span>
          <span className="analysis-panel__metric-label">Critical Path</span>
        </div>
        <div className="analysis-panel__metric">
          <span className="analysis-panel__metric-value">{analysis.max_parallelism}</span>
          <span className="analysis-panel__metric-label">Max Parallelism</span>
        </div>
        <div className="analysis-panel__metric">
          <span className="analysis-panel__metric-value">{speedup}x</span>
          <span className="analysis-panel__metric-label">Speedup</span>
        </div>
      </div>

      <div className="analysis-panel__section">
        <div className="analysis-panel__section-title">Entry / Exit Nodes</div>
        <div className="analysis-panel__nodes-row">
          {analysis.entry_nodes.map((id) => (
            <button key={`e-${id}`} type="button" className="analysis-panel__node-badge analysis-panel__node-badge--entry" onClick={() => setSelectedNodeId(id)}>
              #{id} entry
            </button>
          ))}
          {analysis.exit_nodes.map((id) => (
            <button key={`x-${id}`} type="button" className="analysis-panel__node-badge analysis-panel__node-badge--exit" onClick={() => setSelectedNodeId(id)}>
              #{id} exit
            </button>
          ))}
        </div>
      </div>

      {analysis.critical_path.length > 0 ? (
        <div className="analysis-panel__section">
          <div className="analysis-panel__section-title">Critical Path</div>
          <div className="analysis-panel__nodes-row">
            {analysis.critical_path.map((id, i) => (
              <span key={id}>
                <button type="button" className="analysis-panel__node-badge" onClick={() => setSelectedNodeId(id)}>
                  #{id}
                </button>
                {i < analysis.critical_path.length - 1 ? <span className="analysis-panel__arrow">&rarr;</span> : null}
              </span>
            ))}
          </div>
        </div>
      ) : null}

      {histogramEntries.length > 0 ? (
        <div className="analysis-panel__section">
          <div className="analysis-panel__section-title">Op Histogram</div>
          <div className="analysis-panel__histogram">
            {histogramEntries.map(([op, count]) => (
              <div key={op} className="analysis-panel__histogram-row">
                <span className="analysis-panel__histogram-label">{op}</span>
                <div className="analysis-panel__histogram-bar-wrap">
                  <div
                    className="analysis-panel__histogram-bar"
                    style={{ width: `${(count / maxHistCount) * 100}%` }}
                  />
                </div>
                <span className="analysis-panel__histogram-count">{count}</span>
              </div>
            ))}
          </div>
        </div>
      ) : null}

      {passes.length > 0 ? (
        <div className="analysis-panel__section">
          <div className="analysis-panel__section-title">Compiler Passes ({passes.length})</div>
          <div className="analysis-panel__passes">
            {passes.map((pass) => (
              <div key={pass.name} className="analysis-panel__pass">
                <div className="analysis-panel__pass-header">
                  <span className="analysis-panel__pass-name">{pass.name}</span>
                  <span className="analysis-panel__pass-category">{pass.category}</span>
                </div>
                <div className="analysis-panel__pass-summary">{pass.summary}</div>
              </div>
            ))}
          </div>
        </div>
      ) : null}
    </div>
  );
}
