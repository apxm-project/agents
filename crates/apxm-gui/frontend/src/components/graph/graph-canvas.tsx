import { useMemo, useCallback, useState } from "react";
import {
  ReactFlow,
  Controls,
  MiniMap,
  Background,
  ReactFlowProvider,
  type NodeTypes,
  type EdgeTypes,
  type ReactFlowInstance,
} from "@xyflow/react";
import { useAppStore } from "@/store/app-store";
import { useElkLayout } from "@/hooks/use-elk-layout";
import { buildGraphNodes, buildGraphEdges, getUniqueCategories } from "@/lib/graph-builder";
import { AisNodeCard } from "./ais-node-card";
import { DepEdge } from "./dep-edge";
import { GraphLegend } from "./graph-legend";
import { GraphToolbar } from "./graph-toolbar";
import { GraphAnalysisStrip } from "./graph-analysis";
import type { AisNodeData } from "@/types/graph";

const nodeTypes: NodeTypes = { aisNode: AisNodeCard };
const edgeTypes: EdgeTypes = { depEdge: DepEdge };
const MINIMAP_STYLE = { background: "rgba(13, 19, 28, 0.96)" };
const FIT_VIEW_OPTIONS = { padding: 0.5, maxZoom: 1.0 };

function GraphCanvasInner() {
  const graphData = useAppStore((s) => s.graphData);
  const ops = useAppStore((s) => s.ops);
  const selectedNodeId = useAppStore((s) => s.selectedNodeId);
  const setSelectedNodeId = useAppStore((s) => s.setSelectedNodeId);
  const layoutDirection = useAppStore((s) => s.layoutDirection);
  const setGraphLayout = useAppStore((s) => s.setGraphLayout);
  const liveNodeStates = useAppStore((s) => s.liveNodeStates);
  const [, setFlowInstance] = useState<ReactFlowInstance | null>(null);

  const layout = useElkLayout(graphData, layoutDirection);

  // Sync layout to store
  useMemo(() => { setGraphLayout(layout); }, [layout, setGraphLayout]);

  const graphNodes = useMemo(() => {
    if (!graphData) return [];
    const nodes = buildGraphNodes(graphData, ops, layout, selectedNodeId);
    // Overlay live status
    if (Object.keys(liveNodeStates).length > 0) {
      for (const node of nodes) {
        const status = liveNodeStates[node.data.nodeId];
        if (status) {
          (node.data as AisNodeData).liveStatus = status;
        }
      }
    }
    return nodes;
  }, [graphData, ops, layout, selectedNodeId, liveNodeStates]);

  const graphEdges = useMemo(() => {
    if (!graphData) return [];
    return buildGraphEdges(graphData, layout);
  }, [graphData, layout]);

  const categories = useMemo(() => getUniqueCategories(graphNodes), [graphNodes]);

  const handleNodeClick = useCallback(
    (_: unknown, node: { id: string }) => setSelectedNodeId(Number(node.id)),
    [setSelectedNodeId],
  );

  if (!graphData) {
    return (
      <div className="app-shell">
        <GraphToolbar />
        <div className="viewer-status">
          <p className="viewer-status__title">No graph loaded</p>
          <p className="viewer-status__hint">Browse server files or select a workflow to get started.</p>
        </div>
      </div>
    );
  }

  return (
    <>
      <GraphToolbar />
      <div className="viewer-layout">
        <div className="stage">
          <div className="canvas-card">
            <div className="canvas-card__flow">
              <ReactFlow
                nodes={graphNodes}
                edges={graphEdges}
                nodeTypes={nodeTypes}
                edgeTypes={edgeTypes}
                defaultViewport={{ x: 0, y: 0, zoom: 0.84 }}
                nodesDraggable={false}
                nodesConnectable={false}
                onInit={setFlowInstance}
                onNodeClick={handleNodeClick}
                fitView
                fitViewOptions={FIT_VIEW_OPTIONS}
                proOptions={{ hideAttribution: true }}
              >
                <Controls showInteractive={false} />
                <MiniMap style={MINIMAP_STYLE} />
                <Background color="rgba(148, 163, 184, 0.08)" gap={40} />
              </ReactFlow>
            </div>
            <GraphLegend categories={categories} />
          </div>
        </div>
      </div>
      <GraphAnalysisStrip />
    </>
  );
}

export function GraphCanvas() {
  return (
    <ReactFlowProvider>
      <GraphCanvasInner />
    </ReactFlowProvider>
  );
}
