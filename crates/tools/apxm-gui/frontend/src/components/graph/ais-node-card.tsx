import { memo } from "react";
import { Handle, Position } from "@xyflow/react";
import type { AisNodeData } from "@/types/graph";
import type { NodeLiveStatus } from "@/types/events";
import { getCategoryStyle } from "@/lib/category-styles";
import { truncate, STATUS_COLORS, FALLBACK_STATUS_COLOR } from "@/lib/format";

type AisNodeCardProps = {
  data: AisNodeData;
  selected?: boolean;
};

const LIVE_STATUS_RING: Record<NodeLiveStatus, string> = {
  pending: "rgba(139, 148, 158, 0.3)",
  running: "rgba(62, 166, 198, 0.6)",
  completed: "rgba(88, 187, 129, 0.4)",
  failed: "rgba(216, 107, 107, 0.5)",
};

export const AisNodeCard = memo(function AisNodeCard({ data, selected = false }: AisNodeCardProps) {
  const style = getCategoryStyle(data.category);
  const selectedClass = selected || data.selected ? " ais-node-card--selected" : "";
  const dimmedClass = data.dimmed ? " ais-node-card--dimmed" : "";
  const attrEntries = Object.entries(data.attributes).slice(0, 2);

  const liveRing = data.liveStatus ? LIVE_STATUS_RING[data.liveStatus] : undefined;
  const liveColor = data.liveStatus ? STATUS_COLORS[data.liveStatus] ?? FALLBACK_STATUS_COLOR : undefined;

  return (
    <div
      className={`ais-node-card${selectedClass}${dimmedClass}`}
      style={{
        borderTopColor: style.color,
        background: `linear-gradient(180deg, ${style.gradient}, rgba(18, 25, 35, 0.98) 26%), rgba(18, 25, 35, 0.98)`,
        boxShadow: liveRing ? `0 12px 28px rgba(0,0,0,0.26), 0 0 0 2px ${liveRing}` : undefined,
      }}
    >
      <Handle id="in-top" type="target" position={Position.Top} className="ais-node-card__handle" />
      <Handle id="in-left" type="target" position={Position.Left} className="ais-node-card__handle ais-node-card__handle--side" />
      <Handle id="in-right" type="target" position={Position.Right} className="ais-node-card__handle ais-node-card__handle--side" />

      <div className="ais-node-card__eyebrow">
        <div className="ais-node-card__badges">
          <span className="ais-node-card__category" style={{ background: style.bg, color: style.color }}>
            {data.category.toUpperCase()}
          </span>
          <span className="ais-node-card__op">{data.op}</span>
          {data.isEntry ? <span className="ais-node-card__semantic">entry</span> : null}
          {data.isTerminal ? <span className="ais-node-card__semantic">exit</span> : null}
        </div>
        <span className={`ais-node-card__latency ais-node-card__latency--${data.latency.toLowerCase()}`}>
          {formatLatency(data.latency)}
        </span>
      </div>

      <div className="ais-node-card__title">{data.name}</div>
      <div className="ais-node-card__subtitle">#{data.nodeId}</div>

      {attrEntries.length > 0 ? (
        <div className="ais-node-card__attrs">
          {attrEntries.map(([key, value]) => (
            <span key={key} className="ais-node-card__attr">
              {key}: {truncate(String(value), 32)}
            </span>
          ))}
        </div>
      ) : null}

      <div className="ais-node-card__meta">
        <span>{data.incomingCount} in</span>
        {liveColor ? (
          <span className="ais-node-card__live-dot" style={{ background: liveColor }} />
        ) : null}
        <span>{data.outgoingCount} out</span>
      </div>

      <Handle id="out-bottom" type="source" position={Position.Bottom} className="ais-node-card__handle" />
      <Handle id="out-left" type="source" position={Position.Left} className="ais-node-card__handle ais-node-card__handle--side" />
      <Handle id="out-right" type="source" position={Position.Right} className="ais-node-card__handle ais-node-card__handle--side" />
    </div>
  );
});

function formatLatency(latency: string): string {
  switch (latency.toLowerCase()) {
    case "low": return "LOW";
    case "medium": return "MED";
    case "high": return "HIGH";
    case "none": return "NONE";
    default: return latency.toUpperCase();
  }
}
