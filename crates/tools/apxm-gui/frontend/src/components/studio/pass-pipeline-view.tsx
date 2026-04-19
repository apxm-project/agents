import { useState } from "react";
import type { PassMetricEntry, PassSummary } from "@/types/api";

type Props = {
  metrics: PassMetricEntry[];
  summary: PassSummary | null;
};

const CATEGORY_CLASSES: Record<string, string> = {
  Transform: "pass-pipeline__cat--transform",
  Optimization: "pass-pipeline__cat--optimization",
  Analysis: "pass-pipeline__cat--analysis",
  Lowering: "pass-pipeline__cat--lowering",
};

function formatDuration(ms: number | null): string {
  if (ms == null) return "";
  if (ms < 0.01) return "<0.01ms";
  if (ms < 1) return `${ms.toFixed(2)}ms`;
  if (ms < 100) return `${ms.toFixed(1)}ms`;
  return `${Math.round(ms)}ms`;
}

export function PassPipelineView({ metrics, summary }: Props) {
  const [expandedIdx, setExpandedIdx] = useState<number | null>(null);

  if (metrics.length === 0) return null;

  const maxOps = summary?.initial_ops
    ?? Math.max(...metrics.map((m) => m.ops_before ?? 0), 1);

  const hasTimingData = metrics.some((m) => m.duration_ms != null);
  const activeCount = summary?.active_passes.length
    ?? metrics.filter((m) => m.ops_delta != null && m.ops_delta !== 0).length;

  return (
    <div className="pass-pipeline">
      <div className="pass-pipeline__header">
        <span className="pass-pipeline__title">Pass Pipeline</span>
        <span className="pass-pipeline__stats">
          {metrics.length} passes
          {hasTimingData && summary && (
            <> &middot; {formatDuration(summary.total_duration_ms)}</>
          )}
          {summary && summary.total_ops_eliminated > 0 && (
            <> &middot; {summary.total_ops_eliminated} ops eliminated</>
          )}
          {activeCount > 0 && (
            <> &middot; {activeCount} active</>
          )}
        </span>
      </div>

      <div className="pass-pipeline__steps">
        {metrics.map((m, i) => {
          const isActive = m.ops_delta != null && m.ops_delta !== 0;
          const isExpanded = expandedIdx === i;
          const barWidth = maxOps > 0 && m.ops_after != null
            ? Math.max(4, (m.ops_after / maxOps) * 100)
            : 100;

          return (
            <div
              key={i}
              className={`pass-pipeline__step${isActive ? " pass-pipeline__step--active" : ""}`}
              onClick={() => setExpandedIdx(isExpanded ? null : i)}
            >
              <div className="pass-pipeline__flow-line" />

              <span className="pass-pipeline__index">{i + 1}</span>

              <div className="pass-pipeline__info">
                <div className="pass-pipeline__name-row">
                  <span className="pass-pipeline__name">{m.pass_name}</span>
                  {m.category && (
                    <span className={`pass-pipeline__cat ${CATEGORY_CLASSES[m.category] ?? ""}`}>
                      {m.category}
                    </span>
                  )}
                  {m.duration_ms != null && (
                    <span className="pass-pipeline__timing">
                      {formatDuration(m.duration_ms)}
                    </span>
                  )}
                  {isActive && <span className="pass-pipeline__active-dot" />}
                </div>

                <div className="pass-pipeline__summary">{m.summary}</div>

                {m.ops_after != null && (
                  <div className="pass-pipeline__ops-row">
                    <div className="pass-pipeline__ops-track">
                      <div
                        className={`pass-pipeline__ops-bar${isActive ? " pass-pipeline__ops-bar--active" : ""}`}
                        style={{ width: `${barWidth}%` }}
                      />
                    </div>
                    <span className="pass-pipeline__ops-count">{m.ops_after}</span>
                    {m.ops_delta != null && m.ops_delta !== 0 && (
                      <span className={`pass-pipeline__delta${m.ops_delta < 0 ? " pass-pipeline__delta--neg" : " pass-pipeline__delta--pos"}`}>
                        {m.ops_delta > 0 ? `+${m.ops_delta}` : m.ops_delta}
                      </span>
                    )}
                  </div>
                )}

                {isExpanded && m.description && (
                  <pre className="pass-pipeline__description">{m.description}</pre>
                )}
              </div>
            </div>
          );
        })}
      </div>
    </div>
  );
}
