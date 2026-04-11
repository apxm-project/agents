import { useMemo, useEffect, useCallback, useState, useRef } from "react";
import {
  ReactFlow,
  Controls,
  Background,
  ReactFlowProvider,
  type NodeTypes,
  type EdgeTypes,
  type ReactFlowInstance,
  type Connection,
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
  const editMode = useAppStore((s) => s.editMode);
  const addNode = useAppStore((s) => s.addNode);
  const addEdge = useAppStore((s) => s.addEdge);
  const removeNode = useAppStore((s) => s.removeNode);
  const removeEdge = useAppStore((s) => s.removeEdge);
  const [, setFlowInstance] = useState<ReactFlowInstance | null>(null);
  const [analysisOpen, setAnalysisOpen] = useState(false);
  const reactFlowWrapper = useRef<HTMLDivElement>(null);

  const { layout, layoutError } = useElkLayout(graphData, layoutDirection);

  useEffect(() => { setGraphLayout(layout); }, [layout, setGraphLayout]);

  useEffect(() => {
    if (graphAnalysis) setAnalysisOpen(true);
  }, [graphAnalysis]);

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
      if (searchMatchIds && !searchMatchIds.has(node.data.nodeId)) {
        (node.data as AisNodeData).dimmed = true;
      }
      const status = liveNodeStates[node.data.nodeId];
      if (status) {
        (node.data as AisNodeData).liveStatus = status;
      }
      if (editMode) {
        node.draggable = true;
      }
    }
    return nodes;
  }, [graphData, ops, layout, selectedNodeId, liveNodeStates, searchMatchIds, editMode]);

  const graphEdges = useMemo(() => {
    if (!graphData) return [];
    return buildGraphEdges(graphData, layout);
  }, [graphData, layout]);

  const categories = useMemo(() => getUniqueCategories(graphNodes), [graphNodes]);

  const handleNodeClick = useCallback(
    (_: unknown, node: { id: string }) => setSelectedNodeId(Number(node.id)),
    [setSelectedNodeId],
  );

  const handleConnect = useCallback(
    (connection: Connection) => {
      if (!editMode) return;
      const from = Number(connection.source);
      const to = Number(connection.target);
      if (!isNaN(from) && !isNaN(to) && from !== to) {
        addEdge(from, to, "Data");
      }
    },
    [editMode, addEdge],
  );

  const handleNodesDelete = useCallback(
    (deleted: { id: string }[]) => {
      if (!editMode) return;
      for (const node of deleted) {
        removeNode(Number(node.id));
      }
    },
    [editMode, removeNode],
  );

  const handleEdgesDelete = useCallback(
    (deleted: { id: string }[]) => {
      if (!editMode) return;
      for (const edge of deleted) {
        const [from, to] = edge.id.split("->").map(Number);
        if (!isNaN(from) && !isNaN(to)) {
          removeEdge(from, to);
        }
      }
    },
    [editMode, removeEdge],
  );

  // Drag-and-drop from op palette
  const handleDragOver = useCallback((e: React.DragEvent) => {
    e.preventDefault();
    e.dataTransfer.dropEffect = "copy";
  }, []);

  const handleDrop = useCallback(
    (e: React.DragEvent) => {
      e.preventDefault();
      if (!editMode) return;

      const data = e.dataTransfer.getData("application/apxm-op");
      if (!data) return;

      try {
        const opData = JSON.parse(data) as { name: string; op_type: string; category: string };
        const nodeName = opData.name.toLowerCase().replace(/\s+/g, "_");
        addNode(opData.name, nodeName);
      } catch {
        // ignore invalid drag data
      }
    },
    [editMode, addNode],
  );

  if (!graphData) {
    return (
      <div className="canvas-empty">
        <div className="viewer-status">
          <p className="viewer-status__title">No graph loaded</p>
          <p className="viewer-status__hint">Select a workflow or create a new one to get started.</p>
        </div>
      </div>
    );
  }

  return (
    <>
      <GraphToolbar onToggleAnalysis={() => setAnalysisOpen(!analysisOpen)} />
      {layoutError && (
        <div className="layout-warning">Layout engine unavailable, showing grid fallback</div>
      )}
      <div className="viewer-layout">
        <div className="stage">
          <div
            className="canvas-card"
            ref={reactFlowWrapper}
            onDragOver={handleDragOver}
            onDrop={handleDrop}
          >
            <div className="canvas-card__flow">
              <ReactFlow
                nodes={graphNodes}
                edges={graphEdges}
                nodeTypes={nodeTypes}
                edgeTypes={edgeTypes}
                defaultViewport={{ x: 0, y: 0, zoom: 0.84 }}
                nodesDraggable={editMode}
                nodesConnectable={editMode}
                edgesReconnectable={editMode}
                deleteKeyCode={editMode ? "Delete" : null}
                onInit={setFlowInstance}
                onNodeClick={handleNodeClick}
                onConnect={editMode ? handleConnect : undefined}
                onNodesDelete={editMode ? handleNodesDelete : undefined}
                onEdgesDelete={editMode ? handleEdgesDelete : undefined}
                fitView
                fitViewOptions={FIT_VIEW_OPTIONS}
                proOptions={{ hideAttribution: true }}
              >
                <Controls showInteractive={false} />
                <Background color="rgba(148, 163, 184, 0.08)" gap={40} />
              </ReactFlow>
            </div>
            <GraphLegend categories={categories} />
            {analysisOpen && (
              <GraphAnalysisPanel onClose={() => setAnalysisOpen(false)} />
            )}
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
