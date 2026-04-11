import { create } from "zustand";
import type { ApxmGraph, GraphLayout, GraphNode, GraphEdge, DependencyType } from "@/types/graph";
import type { TraceEvent, NodeLiveStatus } from "@/types/events";
import type { SessionData, SessionInfo } from "@/types/session";
import type { OpSpec } from "@/types/ops";
import type { GraphAnalysis, WorkflowInfo, HealthStatus, PassInfo, CompileResult } from "@/types/api";
import { CtxTab, ViewMode as VM, type OverlayView, type ContextTab, type ViewModeKey } from "@/lib/constants";

type AppState = {
  // UI -- three-column layout
  overlayView: OverlayView;
  contextPanelTab: ContextTab;
  contextPanelOpen: boolean;
  navigatorCollapsed: boolean;
  selectedNodeId: number | null;
  layoutDirection: "DOWN" | "RIGHT";
  inspectorOpen: boolean;
  searchQuery: string;
  // Explorer view mode (used in Studio overlay)
  viewMode: ViewModeKey;
  sourceContent: string | null;
  sourcePath: string | null;
  selectedWorkflow: WorkflowInfo | null;
  // Compile & Execute
  compileResult: CompileResult | null;
  executeDialogOpen: boolean;
  // Graph
  graphData: ApxmGraph | null;
  graphPath: string | null;
  graphLayout: GraphLayout | null;
  graphAnalysis: GraphAnalysis | null;
  // Editor
  editMode: boolean;
  dirty: boolean;
  nextNodeId: number;
  // Session
  sessionData: SessionData | null;
  sessionPath: string | null;
  sessions: SessionInfo[];
  sessionNodeId: number | null;
  // Live
  liveActive: boolean;
  liveSessionPath: string | null;
  liveNodeStates: Record<number, NodeLiveStatus>;
  liveEvents: TraceEvent[];
  liveManifest: Record<string, unknown> | null;
  // Inline execution (stay on graph tab)
  inlineRun: boolean;
  lastRunSessionPath: string | null;
  lastRunStatus: string | null;
  // Live progress from SSE
  liveProgress: Record<string, unknown> | null;
  // Sessions auto-refresh
  sessionsAutoRefresh: boolean;
  // Data
  ops: OpSpec[];
  passes: PassInfo[];
  config: string;
  workflows: WorkflowInfo[];
  health: HealthStatus | null;
  // Loading / errors
  loading: Record<string, boolean>;
  errors: Record<string, string | null>;

  // Actions -- layout
  setOverlayView: (view: OverlayView) => void;
  setContextPanelTab: (tab: ContextTab) => void;
  toggleContextPanel: () => void;
  setNavigatorCollapsed: (collapsed: boolean) => void;
  setSelectedNodeId: (id: number | null) => void;
  setLayoutDirection: (dir: "DOWN" | "RIGHT") => void;
  toggleInspector: () => void;
  setSearchQuery: (query: string) => void;
  setViewMode: (mode: ViewModeKey) => void;
  setSourceContent: (content: string | null, path?: string | null) => void;
  setSelectedWorkflow: (workflow: WorkflowInfo | null) => void;
  selectWorkflowAndShow: (workflow: WorkflowInfo) => void;
  setCompileResult: (result: CompileResult | null) => void;
  setExecuteDialogOpen: (open: boolean) => void;
  setGraphData: (graph: ApxmGraph | null, path?: string | null) => void;
  setGraphLayout: (layout: GraphLayout | null) => void;
  setGraphAnalysis: (analysis: GraphAnalysis | null) => void;
  // Editor actions
  setEditMode: (edit: boolean) => void;
  newGraph: (name: string) => void;
  addNode: (op: string, name: string, attributes?: Record<string, unknown>) => number;
  removeNode: (id: number) => void;
  updateNodeAttr: (id: number, key: string, value: unknown) => void;
  updateNodeName: (id: number, name: string) => void;
  addEdge: (from: number, to: number, dependency?: DependencyType) => void;
  removeEdge: (from: number, to: number) => void;
  moveNode: (id: number, x: number, y: number) => void;
  setDirty: (dirty: boolean) => void;
  // Session actions
  setSessionData: (data: SessionData | null, path?: string | null) => void;
  setSessions: (sessions: SessionInfo[]) => void;
  setSessionNodeId: (id: number | null) => void;
  setLiveActive: (active: boolean, sessionPath?: string | null) => void;
  setLiveNodeStatus: (nodeId: number, status: NodeLiveStatus) => void;
  addLiveEvent: (event: TraceEvent) => void;
  setLiveManifest: (manifest: Record<string, unknown> | null) => void;
  clearLive: () => void;
  startInlineRun: (sessionPath: string) => void;
  finishInlineRun: (status: string) => void;
  setLiveProgress: (progress: Record<string, unknown> | null) => void;
  setSessionsAutoRefresh: (on: boolean) => void;
  navigateToSession: (sessionPath: string) => void;
  setOps: (ops: OpSpec[]) => void;
  setPasses: (passes: PassInfo[]) => void;
  setConfig: (config: string) => void;
  setWorkflows: (workflows: WorkflowInfo[]) => void;
  setHealth: (health: HealthStatus | null) => void;
  setLoading: (key: string, loading: boolean) => void;
  setError: (key: string, error: string | null) => void;
};

const MAX_LIVE_EVENTS = 5000;

export const useAppStore = create<AppState>((set, get) => ({
  overlayView: null,
  contextPanelTab: CtxTab.GRAPH,
  contextPanelOpen: true,
  navigatorCollapsed: false,
  selectedNodeId: null,
  layoutDirection: "DOWN",
  inspectorOpen: true,
  searchQuery: "",
  viewMode: VM.SOURCE,
  sourceContent: null,
  sourcePath: null,
  selectedWorkflow: null,
  compileResult: null,
  executeDialogOpen: false,
  graphData: null,
  graphPath: null,
  graphLayout: null,
  graphAnalysis: null,
  editMode: false,
  dirty: false,
  nextNodeId: 1,
  sessionData: null,
  sessionPath: null,
  sessions: [],
  sessionNodeId: null,
  liveActive: false,
  liveSessionPath: null,
  liveNodeStates: {},
  liveEvents: [],
  liveManifest: null,
  inlineRun: false,
  lastRunSessionPath: null,
  lastRunStatus: null,
  liveProgress: null,
  sessionsAutoRefresh: true,
  ops: [],
  passes: [],
  config: "",
  workflows: [],
  health: null,
  loading: {},
  errors: {},

  setOverlayView: (view) => set({ overlayView: view }),
  setContextPanelTab: (tab) => set({ contextPanelTab: tab, contextPanelOpen: true }),
  toggleContextPanel: () => set((s) => ({ contextPanelOpen: !s.contextPanelOpen })),
  setNavigatorCollapsed: (collapsed) => set({ navigatorCollapsed: collapsed }),
  setSelectedNodeId: (id) => set({ selectedNodeId: id }),
  setLayoutDirection: (dir) => set({ layoutDirection: dir }),
  toggleInspector: () => set((s) => ({ inspectorOpen: !s.inspectorOpen })),
  setSearchQuery: (query) => set({ searchQuery: query }),
  setViewMode: (mode) => set({ viewMode: mode }),
  setSourceContent: (content, path) => set({ sourceContent: content, sourcePath: path ?? null }),
  setSelectedWorkflow: (workflow) => set({ selectedWorkflow: workflow }),
  setCompileResult: (result) => set({ compileResult: result }),
  setExecuteDialogOpen: (open) => set({ executeDialogOpen: open }),
  setGraphData: (graph, path) => {
    const maxId = graph ? Math.max(0, ...graph.nodes.map((n) => n.id)) : 0;
    set({
      graphData: graph, graphPath: path ?? null, selectedNodeId: null,
      graphAnalysis: null, graphLayout: null, searchQuery: "",
      nextNodeId: maxId + 1, dirty: false,
    });
  },
  setGraphLayout: (layout) => set({ graphLayout: layout }),
  setGraphAnalysis: (analysis) => set({ graphAnalysis: analysis }),

  // ── Editor actions ──────────────────────────────────────────
  setEditMode: (edit) => set({ editMode: edit }),

  newGraph: (name) => set({
    graphData: { name, nodes: [], edges: [], parameters: [], metadata: {} },
    graphPath: null,
    graphLayout: null,
    graphAnalysis: null,
    selectedNodeId: null,
    searchQuery: "",
    viewMode: VM.GRAPH,
    editMode: true,
    dirty: true,
    nextNodeId: 1,
    compileResult: null,
  }),

  addNode: (op, name, attributes = {}) => {
    const id = get().nextNodeId;
    const node: GraphNode = { id, name, op, attributes };
    set((s) => ({
      graphData: s.graphData
        ? { ...s.graphData, nodes: [...s.graphData.nodes, node] }
        : { name: "untitled", nodes: [node], edges: [], parameters: [], metadata: {} },
      nextNodeId: id + 1,
      dirty: true,
      graphLayout: null,
    }));
    return id;
  },

  removeNode: (id) => set((s) => {
    if (!s.graphData) return s;
    return {
      graphData: {
        ...s.graphData,
        nodes: s.graphData.nodes.filter((n) => n.id !== id),
        edges: s.graphData.edges.filter((e) => e.from !== id && e.to !== id),
      },
      selectedNodeId: s.selectedNodeId === id ? null : s.selectedNodeId,
      dirty: true,
      graphLayout: null,
    };
  }),

  updateNodeAttr: (id, key, value) => set((s) => {
    if (!s.graphData) return s;
    return {
      graphData: {
        ...s.graphData,
        nodes: s.graphData.nodes.map((n) =>
          n.id === id ? { ...n, attributes: { ...n.attributes, [key]: value } } : n
        ),
      },
      dirty: true,
    };
  }),

  updateNodeName: (id, name) => set((s) => {
    if (!s.graphData) return s;
    return {
      graphData: {
        ...s.graphData,
        nodes: s.graphData.nodes.map((n) =>
          n.id === id ? { ...n, name } : n
        ),
      },
      dirty: true,
    };
  }),

  addEdge: (from, to, dependency = "Data") => set((s) => {
    if (!s.graphData) return s;
    const exists = s.graphData.edges.some((e) => e.from === from && e.to === to);
    if (exists) return s;
    const edge: GraphEdge = { from, to, dependency };
    return {
      graphData: { ...s.graphData, edges: [...s.graphData.edges, edge] },
      dirty: true,
      graphLayout: null,
    };
  }),

  removeEdge: (from, to) => set((s) => {
    if (!s.graphData) return s;
    return {
      graphData: {
        ...s.graphData,
        edges: s.graphData.edges.filter((e) => !(e.from === from && e.to === to)),
      },
      dirty: true,
      graphLayout: null,
    };
  }),

  moveNode: (_id, _x, _y) => {
    // Position changes are handled by React Flow's internal state and ELK layout
    set({ dirty: true });
  },

  setDirty: (dirty) => set({ dirty }),

  // ── Session / other actions ─────────────────────────────────
  setSessionData: (data, path) => set({ sessionData: data, sessionPath: path ?? null }),
  setSessions: (sessions) => set({ sessions }),
  setSessionNodeId: (id) => set({ sessionNodeId: id }),
  setLiveActive: (active, sessionPath) => set({ liveActive: active, liveSessionPath: sessionPath ?? null }),
  setLiveNodeStatus: (nodeId, status) =>
    set((s) => ({ liveNodeStates: { ...s.liveNodeStates, [nodeId]: status } })),
  addLiveEvent: (event) =>
    set((s) => {
      const events = s.liveEvents.length >= MAX_LIVE_EVENTS
        ? [...s.liveEvents.slice(-(MAX_LIVE_EVENTS - 1)), event]
        : [...s.liveEvents, event];
      return { liveEvents: events };
    }),
  setLiveManifest: (manifest) => set({ liveManifest: manifest }),
  clearLive: () => set({ liveActive: false, liveSessionPath: null, liveNodeStates: {}, liveEvents: [], liveManifest: null, liveProgress: null, inlineRun: false }),
  startInlineRun: (sessionPath) => set({
    liveActive: true, liveSessionPath: sessionPath,
    liveNodeStates: {}, liveEvents: [], liveManifest: null,
    inlineRun: true, lastRunSessionPath: sessionPath, lastRunStatus: null,
  }),
  finishInlineRun: (status) => set({ lastRunStatus: status }),
  setLiveProgress: (progress) => set({ liveProgress: progress }),
  setSessionsAutoRefresh: (on) => set({ sessionsAutoRefresh: on }),
  selectWorkflowAndShow: (workflow) => set({
    selectedWorkflow: workflow,
    contextPanelOpen: true,
    contextPanelTab: CtxTab.GRAPH,
    overlayView: null,
  }),
  navigateToSession: (sessionPath) => set({
    liveActive: true, liveSessionPath: sessionPath,
    liveNodeStates: {}, liveEvents: [], liveManifest: null, liveProgress: null,
    contextPanelTab: CtxTab.LIVE, contextPanelOpen: true, overlayView: null,
    inlineRun: false, sessionPath: sessionPath,
  }),
  setOps: (ops) => set({ ops }),
  setPasses: (passes) => set({ passes }),
  setConfig: (config) => set({ config }),
  setWorkflows: (workflows) => set({ workflows }),
  setHealth: (health) => set({ health }),
  setLoading: (key, loading) => set((s) => ({ loading: { ...s.loading, [key]: loading } })),
  setError: (key, error) => set((s) => ({ errors: { ...s.errors, [key]: error } })),
}));
