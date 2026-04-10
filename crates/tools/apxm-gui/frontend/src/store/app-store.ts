import { create } from "zustand";
import type { ApxmGraph, GraphLayout } from "@/types/graph";
import type { TraceEvent, NodeLiveStatus } from "@/types/events";
import type { SessionData, SessionInfo } from "@/types/session";
import type { OpSpec } from "@/types/ops";
import type { GraphAnalysis, WorkflowInfo, HealthStatus, PassInfo } from "@/types/api";
import type { TabKey } from "@/lib/constants";

type AppState = {
  // UI
  activeTab: TabKey;
  selectedNodeId: number | null;
  layoutDirection: "DOWN" | "RIGHT";
  inspectorOpen: boolean;
  searchQuery: string;
  // Graph
  graphData: ApxmGraph | null;
  graphPath: string | null;
  graphLayout: GraphLayout | null;
  graphAnalysis: GraphAnalysis | null;
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
  // Replay
  replaySessionPath: string | null;
  // Data
  ops: OpSpec[];
  passes: PassInfo[];
  config: string;
  workflows: WorkflowInfo[];
  health: HealthStatus | null;
  // Loading / errors
  loading: Record<string, boolean>;
  errors: Record<string, string | null>;

  // Actions
  setActiveTab: (tab: TabKey) => void;
  setSelectedNodeId: (id: number | null) => void;
  setLayoutDirection: (dir: "DOWN" | "RIGHT") => void;
  toggleInspector: () => void;
  setSearchQuery: (query: string) => void;
  setGraphData: (graph: ApxmGraph | null, path?: string | null) => void;
  setGraphLayout: (layout: GraphLayout | null) => void;
  setGraphAnalysis: (analysis: GraphAnalysis | null) => void;
  setSessionData: (data: SessionData | null, path?: string | null) => void;
  setSessions: (sessions: SessionInfo[]) => void;
  setSessionNodeId: (id: number | null) => void;
  setLiveActive: (active: boolean, sessionPath?: string | null) => void;
  setLiveNodeStatus: (nodeId: number, status: NodeLiveStatus) => void;
  addLiveEvent: (event: TraceEvent) => void;
  setLiveManifest: (manifest: Record<string, unknown> | null) => void;
  clearLive: () => void;
  navigateToLive: (sessionPath: string) => void;
  navigateToReplay: (sessionPath: string) => void;
  setOps: (ops: OpSpec[]) => void;
  setPasses: (passes: PassInfo[]) => void;
  setConfig: (config: string) => void;
  setWorkflows: (workflows: WorkflowInfo[]) => void;
  setHealth: (health: HealthStatus | null) => void;
  setLoading: (key: string, loading: boolean) => void;
  setError: (key: string, error: string | null) => void;
};

const MAX_LIVE_EVENTS = 5000;

export const useAppStore = create<AppState>((set) => ({
  activeTab: "dashboard",
  selectedNodeId: null,
  layoutDirection: "DOWN",
  inspectorOpen: true,
  searchQuery: "",
  graphData: null,
  graphPath: null,
  graphLayout: null,
  graphAnalysis: null,
  sessionData: null,
  sessionPath: null,
  sessions: [],
  sessionNodeId: null,
  liveActive: false,
  liveSessionPath: null,
  liveNodeStates: {},
  liveEvents: [],
  liveManifest: null,
  replaySessionPath: null,
  ops: [],
  passes: [],
  config: "",
  workflows: [],
  health: null,
  loading: {},
  errors: {},

  setActiveTab: (tab) => set({ activeTab: tab }),
  setSelectedNodeId: (id) => set({ selectedNodeId: id }),
  setLayoutDirection: (dir) => set({ layoutDirection: dir }),
  toggleInspector: () => set((s) => ({ inspectorOpen: !s.inspectorOpen })),
  setSearchQuery: (query) => set({ searchQuery: query }),
  setGraphData: (graph, path) => set({
    graphData: graph, graphPath: path ?? null, selectedNodeId: null,
    graphAnalysis: null, graphLayout: null, searchQuery: "",
  }),
  setGraphLayout: (layout) => set({ graphLayout: layout }),
  setGraphAnalysis: (analysis) => set({ graphAnalysis: analysis }),
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
  clearLive: () => set({ liveActive: false, liveSessionPath: null, liveNodeStates: {}, liveEvents: [], liveManifest: null }),
  navigateToLive: (sessionPath) => set({ liveActive: true, liveSessionPath: sessionPath, activeTab: "live" }),
  navigateToReplay: (sessionPath) => set({ replaySessionPath: sessionPath, activeTab: "replay" }),
  setOps: (ops) => set({ ops }),
  setPasses: (passes) => set({ passes }),
  setConfig: (config) => set({ config }),
  setWorkflows: (workflows) => set({ workflows }),
  setHealth: (health) => set({ health }),
  setLoading: (key, loading) => set((s) => ({ loading: { ...s.loading, [key]: loading } })),
  setError: (key, error) => set((s) => ({ errors: { ...s.errors, [key]: error } })),
}));
