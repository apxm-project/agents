import { BaseEdge, type EdgeProps } from "@xyflow/react";
import type { DepEdgeData } from "@/types/graph";
import { getEdgeStyle } from "@/lib/category-styles";

export function DepEdge({ id, data, markerEnd, sourceX, sourceY, targetX, targetY }: EdgeProps) {
  const edgeData = data as DepEdgeData | undefined;
  const dependency = edgeData?.dependency ?? "Data";
  const edgeStyle = getEdgeStyle(dependency);
  const points = edgeData?.points && edgeData.points.length >= 2
    ? edgeData.points
    : [{ x: sourceX, y: sourceY }, { x: targetX, y: targetY }];

  let path = `M ${points[0].x} ${points[0].y}`;
  for (let i = 1; i < points.length; i++) {
    path += ` L ${points[i].x} ${points[i].y}`;
  }

  return (
    <BaseEdge
      id={id}
      path={path}
      markerEnd={markerEnd}
      style={{ stroke: edgeStyle.stroke, strokeWidth: 1.8, strokeDasharray: edgeStyle.strokeDasharray }}
    />
  );
}
