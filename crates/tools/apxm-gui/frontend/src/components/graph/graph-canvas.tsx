import { useMemo, useEffect, useCallback, useState } from "react";
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
import { GraphAnalysisPanel } from "./graph-analysis";
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
  const searchQuery = useAppStore((s) => s.searchQuery);
  const graphAnalysis = useAppStore((s) => s.graphAnalysis);
  const [, setFlowInstance] = useState<ReactFlowInstance | null>(null);
  const [analysisOpen, setAnalysisOpen] = useState(false);

  const { layout, layoutError } = useElkLayout(graphData, layoutDirection);

  // Sync layout to store
  useEffect(() => { setGraphLayout(layout); }, [layout, setGraphLayout]);

  // Auto-open analysis panel when analysis arrives
  useEffect(() => {
    if (graphAnalysis) setAnalysisOpen(true);
  }, [graphAnalysis]);

  // Compute search match set
  const searchMatchIds = useMemo(() => {
    if (!searchQuery || !graphData) return null;
    const q = searchQuery.toLowerCase();
    const ids = new Set<number>();
    for (const node of graphData.nodes) {
      if (
        node.name.toLowerCase().includes(q) ||
        node.op.toLowerCase().includes(q) ||
        Object.values(node.attributes).some((v) => String(v).toLowerCase().includes(q))
      ) {
        ids.add(node.id);
      }
    }
    return ids;
  }, [searchQuery, graphData]);

  const graphNodes = useMemo(() => {
    if (!graphData) return [];
    const nodes = buildGraphNodes(graphData, ops, layout, selectedNodeId);
    for (const node of nodes) {
      // Apply search dimming
      if (searchMatchIds && !searchMatchIds.has(node.data.nodeId)) {
        (node.data as AisNodeData).dimmed = true;
      }
      // Overlay live status
      const status = liveNodeStates[node.data.nodeId];
      if (status) {
        (node.data as AisNodeData).liveStatus = status;
      }
    }
    return nodes;
  }, [graphData, ops, layout, selectedNodeId, liveNodeStates, searchMatchIds]);

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
      <div className="canvas-empty">
        <div className="viewer-status">
          <p className="viewer-status__title">No graph loaded</p>
          <p className="viewer-status__hint">Select a .apxm file from the sidebar to visualize it.</p>
        </div>
      </div>
    );
  }

  return (
    <>
      <GraphToolbar onToggleAnalysis={() => setAnalysisOpen(!analysisOpen)} />
      {layoutError ? (
        <div className="layout-warning">Layout engine unavailable, showing grid fallback</div>
      ) : null}
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
            {analysisOpen ? (
              <GraphAnalysisPanel onClose={() => setAnalysisOpen(false)} />
            ) : null}
          </div>
        </div>
      </div>
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
