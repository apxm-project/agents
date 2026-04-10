import { Position, type Edge, type Node } from "@xyflow/react";
import type { ElkNode, ElkExtendedEdge } from "elkjs/lib/elk.bundled.js";
import type { ApxmGraph, GraphLayout, AisNodeData, DepEdgeData, ViewerPoint } from "@/types/graph";
import type { OpSpec } from "@/types/ops";
import { getEdgeStyle } from "./category-styles";
import { ELK_NODE_WIDTH, ELK_NODE_HEIGHT, FALLBACK_GRID_SPACING, elkLayoutOptionsForDirection } from "./layout-config";

export function buildGraphNodes(
  graph: ApxmGraph,
  opMetadata: OpSpec[],
  layout: GraphLayout | null,
  selectedNodeId: number | null,
): Node<AisNodeData>[] {
  const opLookup = new Map(opMetadata.map((op) => [op.name, op]));
  const incomingCounts = new Map<number, number>();
  const outgoingCounts = new Map<number, number>();

  for (const edge of graph.edges) {
    incomingCounts.set(edge.to, (incomingCounts.get(edge.to) ?? 0) + 1);
    outgoingCounts.set(edge.from, (outgoingCounts.get(edge.from) ?? 0) + 1);
  }

  const hasIncoming = new Set(graph.edges.map((e) => e.to));
  const hasOutgoing = new Set(graph.edges.map((e) => e.from));

  return graph.nodes.map((node, index) => {
    const op = opLookup.get(node.op);
    const category = op?.category ?? "unknown";
    const latency = op?.latency ?? "unknown";
    const description = op?.description ?? "";
    const producesOutput = op?.produces_output ?? true;
    const isEntry = !hasIncoming.has(node.id);
    const isTerminal = !hasOutgoing.has(node.id);

    const layoutPos = layout?.nodePositions[String(node.id)];
    const position = layoutPos ?? fallbackPosition(index);

    return {
      id: String(node.id),
      type: "aisNode",
      data: {
        nodeId: node.id,
        name: node.name,
        op: node.op,
        category,
        latency,
        description,
        producesOutput,
        attributes: node.attributes,
        isEntry,
        isTerminal,
        incomingCount: incomingCounts.get(node.id) ?? 0,
        outgoingCount: outgoingCounts.get(node.id) ?? 0,
        selected: node.id === selectedNodeId,
      },
      position: { x: position.x, y: position.y },
      sourcePosition: Position.Bottom,
      targetPosition: Position.Top,
      draggable: false,
      selectable: true,
    } satisfies Node<AisNodeData>;
  });
}

export function buildGraphEdges(
  graph: ApxmGraph,
  layout: GraphLayout | null,
): Edge<DepEdgeData>[] {
  return graph.edges.map((edge) => {
    const edgeId = `${edge.from}->${edge.to}`;
    const edgeStyle = getEdgeStyle(edge.dependency);
    const routedPoints = layout?.edgeRoutes[edgeId]?.points;

    return {
      id: edgeId,
      source: String(edge.from),
      target: String(edge.to),
      type: "depEdge",
      data: {
        dependency: edge.dependency,
        points: routedPoints,
      },
      style: {
        stroke: edgeStyle.stroke,
        strokeWidth: 1.8,
        strokeDasharray: edgeStyle.strokeDasharray,
      },
      markerEnd: {
        type: "arrowclosed" as const,
        color: edgeStyle.markerColor,
      },
    } satisfies Edge<DepEdgeData>;
  });
}

export function getUniqueCategories(nodes: Node<AisNodeData>[]): string[] {
  const categories = new Set(nodes.map((n) => n.data.category));
  categories.delete("unknown");
  return Array.from(categories).sort();
}

function fallbackPosition(index: number): ViewerPoint {
  return { x: 0, y: index * FALLBACK_GRID_SPACING.y };
}

// ELK layout computation
let elkPromise: Promise<import("elkjs/lib/elk.bundled.js").ELK> | null = null;

async function getElk() {
  if (!elkPromise) {
    elkPromise = import("elkjs/lib/elk.bundled.js").then(({ default: Elk }) => new Elk());
  }
  return elkPromise;
}

export async function computeElkLayout(
  graph: ApxmGraph,
  direction: "DOWN" | "RIGHT",
): Promise<GraphLayout | null> {
  const elkGraph: ElkNode = {
    id: "root",
    layoutOptions: elkLayoutOptionsForDirection(direction),
    children: graph.nodes.map((node, index) => ({
      id: String(node.id),
      width: ELK_NODE_WIDTH,
      height: ELK_NODE_HEIGHT,
      layoutOptions: { "elk.priority": String(1000 - index) },
    })),
    edges: graph.edges.map((edge, index) => ({
      id: `${edge.from}->${edge.to}`,
      sources: [String(edge.from)],
      targets: [String(edge.to)],
      layoutOptions: { "elk.priority": String(1000 - index) },
    })) satisfies ElkExtendedEdge[],
  };

  try {
    const elk = await getElk();
    const result = await elk.layout(elkGraph);
    const nodePositions: GraphLayout["nodePositions"] = {};
    const edgeRoutes: GraphLayout["edgeRoutes"] = {};

    for (const child of result.children ?? []) {
      nodePositions[child.id] = { x: child.x ?? 0, y: child.y ?? 0 };
    }

    for (const edge of result.edges ?? []) {
      const points = extractElkEdgePoints(edge as ElkExtendedEdge);
      if (points.length > 0) {
        edgeRoutes[edge.id] = { points };
      }
    }

    return { nodePositions, edgeRoutes };
  } catch {
    return null;
  }
}

function extractElkEdgePoints(edge: ElkExtendedEdge): ViewerPoint[] {
  const points: ViewerPoint[] = [];
  for (const section of edge.sections ?? []) {
    if (points.length === 0) points.push(section.startPoint);
    for (const bp of section.bendPoints ?? []) points.push(bp);
    points.push(section.endPoint);
  }
  return points.filter((p, i) => i === 0 || p.x !== points[i - 1]!.x || p.y !== points[i - 1]!.y);
}
