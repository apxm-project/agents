import { useEffect, useRef, useState, useCallback, useMemo } from "react";
import { useAppStore } from "@/store/app-store";
import { fetchWorkflows } from "@/api/workflows";
import { fetchSession, fetchSessions } from "@/api/session";
import { fetchGraph } from "@/api/graph";
import {
  STATUS_COLORS,
  FALLBACK_STATUS_COLOR,
  statusBadgeStyle,
  formatDuration,
  timeAgo,
  isStaleSession,
  sessionPath,
} from "@/lib/format";
import { Overlay, CtxTab, type OverlayView } from "@/lib/constants";
import type { WorkflowInfo } from "@/types/api";
import type { SessionInfo } from "@/types/session";

const OVERLAY_ITEMS: { key: OverlayView; label: string; icon: string }[] = [
  { key: Overlay.STUDIO, label: "Studio", icon: "M7 21l3-3m0 0l3 3m-3-3v8M4 4h16M4 4v16h16V4M9 9h6M9 9v6m6-6v6M9 15h6" },
  { key: Overlay.REFERENCE, label: "Reference", icon: "M12 6.25a.75.75 0 01.75.75v3.25H16a.75.75 0 010 1.5h-3.25V15a.75.75 0 01-1.5 0v-3.25H8a.75.75 0 010-1.5h3.25V7a.75.75 0 01.75-.75z" },
  { key: Overlay.SETTINGS, label: "Settings", icon: "M10.325 4.317c.426-1.756 2.924-1.756 3.35 0a1.724 1.724 0 002.573 1.066c1.543-.94 3.31.826 2.37 2.37a1.724 1.724 0 001.066 2.573c1.756.426 1.756 2.924 0 3.35a1.724 1.724 0 00-1.066 2.573c.94 1.543-.826 3.31-2.37 2.37a1.724 1.724 0 00-2.573 1.066c-.426 1.756-2.924 1.756-3.35 0a1.724 1.724 0 00-2.573-1.066c-1.543.94-3.31-.826-2.37-2.37a1.724 1.724 0 00-1.066-2.573c-1.756-.426-1.756-2.924 0-3.35a1.724 1.724 0 001.066-2.573c-.94-1.543.826-3.31 2.37-2.37.996.608 2.296.07 2.572-1.065z M15 12a3 3 0 11-6 0 3 3 0 016 0z" },
];

export function NavigatorPanel() {
  const workflows = useAppStore((s) => s.workflows);
  const sessions = useAppStore((s) => s.sessions);
  const selectedWorkflow = useAppStore((s) => s.selectedWorkflow);
  const setWorkflows = useAppStore((s) => s.setWorkflows);
  const setSessions = useAppStore((s) => s.setSessions);
  const selectWorkflowAndShow = useAppStore((s) => s.selectWorkflowAndShow);
  const setOverlayView = useAppStore((s) => s.setOverlayView);
  const navigateToSession = useAppStore((s) => s.navigateToSession);
  const setGraphData = useAppStore((s) => s.setGraphData);
  const setContextPanelTab = useAppStore((s) => s.setContextPanelTab);
  const setSessionData = useAppStore((s) => s.setSessionData);
  const [search, setSearch] = useState("");
  const intervalRef = useRef<ReturnType<typeof setInterval> | null>(null);

  useEffect(() => {
    fetchWorkflows().then(setWorkflows).catch(() => {});
    fetchSessions().then(setSessions).catch(() => {});
  }, [setWorkflows, setSessions]);

  useEffect(() => {
    intervalRef.current = setInterval(() => {
      fetchSessions().then(setSessions).catch(() => {});
    }, 10_000);
    return () => { if (intervalRef.current) clearInterval(intervalRef.current); };
  }, [setSessions]);

  const handleSelectWorkflow = useCallback((w: WorkflowInfo) => {
    selectWorkflowAndShow(w);
    fetchGraph(w.path)
      .then((graph) => setGraphData(graph, w.path))
      .catch(() => {});
  }, [selectWorkflowAndShow, setGraphData]);

  const handleSessionClick = useCallback((s: SessionInfo) => {
    if (s.status === "running") {
      navigateToSession(sessionPath(s.id));
    } else {
      fetchSession(sessionPath(s.id))
        .then((data) => {
          setSessionData(data, sessionPath(s.id));
          setContextPanelTab(CtxTab.SESSIONS);
        })
        .catch(() => {});
    }
  }, [navigateToSession, setSessionData, setContextPanelTab]);

  const filteredWorkflows = useMemo(() => {
    if (!search) return workflows;
    const q = search.toLowerCase();
    return workflows.filter(
      (w) => w.name.toLowerCase().includes(q) || w.category.toLowerCase().includes(q),
    );
  }, [workflows, search]);

  const categories = useMemo(() => {
    const cats = [...new Set(filteredWorkflows.map((w) => w.category))].sort();
    return cats;
  }, [filteredWorkflows]);

  const runningSessions = useMemo(
    () => sessions.filter((s) => s.status === "running"),
    [sessions],
  );

  const recentSessions = useMemo(
    () => sessions.filter((s) => s.status !== "running").slice(0, 5),
    [sessions],
  );

  return (
    <div className="nav-panel">
      <div className="nav-panel__header">
        <span className="nav-panel__logo">APXM</span>
      </div>

      <input
        type="text"
        className="nav-panel__search"
        placeholder="Search workflows..."
        value={search}
        onChange={(e) => setSearch(e.target.value)}
      />

      <div className="nav-panel__body">
        {/* Workflows */}
        <div className="nav-section">
          <div className="nav-section__title">
            <span>Workflows</span>
            <span className="nav-section__count">{workflows.length}</span>
          </div>
          {filteredWorkflows.length === 0 && (
            <div className="nav-section__empty">
              {search ? "No matches" : "No workflows found"}
            </div>
          )}
          {categories.length <= 1
            ? filteredWorkflows.map((w) => (
                <WorkflowItem
                  key={w.path}
                  workflow={w}
                  active={selectedWorkflow?.path === w.path}
                  runningSessions={runningSessions}
                  onSelect={handleSelectWorkflow}
                />
              ))
            : categories.map((cat) => (
                <div key={cat} className="nav-category">
                  <span className="nav-category__label">{cat}</span>
                  {filteredWorkflows
                    .filter((w) => w.category === cat)
                    .map((w) => (
                      <WorkflowItem
                        key={w.path}
                        workflow={w}
                        active={selectedWorkflow?.path === w.path}
                        runningSessions={runningSessions}
                        onSelect={handleSelectWorkflow}
                      />
                    ))}
                </div>
              ))}
        </div>

        {/* Live Sessions */}
        {runningSessions.length > 0 && (
          <div className="nav-section">
            <div className="nav-section__title">
              <span>Live</span>
              <span className="nav-section__count nav-section__count--live">
                {runningSessions.length}
              </span>
            </div>
            {runningSessions.map((s) => (
              <SessionItem key={s.id} session={s} onSelect={handleSessionClick} />
            ))}
          </div>
        )}

        {/* Recent Sessions */}
        {recentSessions.length > 0 && (
          <div className="nav-section">
            <div className="nav-section__title">
              <span>Recent</span>
            </div>
            {recentSessions.map((s) => (
              <SessionItem key={s.id} session={s} onSelect={handleSessionClick} />
            ))}
          </div>
        )}
      </div>

      {/* Footer -- overlay links */}
      <div className="nav-panel__footer">
        {OVERLAY_ITEMS.map((item) => (
          <button
            key={item.key}
            type="button"
            className="nav-footer-btn"
            onClick={() => setOverlayView(item.key)}
            title={item.label}
          >
            <svg className="nav-footer-btn__icon" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.5">
              <path d={item.icon} />
            </svg>
            <span>{item.label}</span>
          </button>
        ))}
      </div>
    </div>
  );
}

function WorkflowItem({
  workflow,
  active,
  runningSessions,
  onSelect,
}: {
  workflow: WorkflowInfo;
  active: boolean;
  runningSessions: SessionInfo[];
  onSelect: (w: WorkflowInfo) => void;
}) {
  const isRunning = runningSessions.some((s) => s.graph_name === workflow.name);

  return (
    <button
      type="button"
      className={`nav-workflow${active ? " nav-workflow--active" : ""}`}
      onClick={() => onSelect(workflow)}
      title={workflow.relative_path}
    >
      <div className="nav-workflow__top">
        {isRunning && <span className="nav-workflow__live-dot" />}
        <span className="nav-workflow__name">{workflow.name}</span>
      </div>
      <div className="nav-workflow__meta">
        {workflow.node_count != null && (
          <span className="nav-workflow__nodes">{workflow.node_count}n</span>
        )}
        <span className="nav-workflow__type">{workflow.source_type}</span>
      </div>
    </button>
  );
}

function SessionItem({
  session,
  onSelect,
}: {
  session: SessionInfo;
  onSelect: (s: SessionInfo) => void;
}) {
  const color = STATUS_COLORS[session.status] ?? FALLBACK_STATUS_COLOR;
  const stale = session.status === "running" && isStaleSession(session.mtime_epoch);

  return (
    <button
      type="button"
      className={`nav-session${stale ? " nav-session--stale" : ""}`}
      onClick={() => onSelect(session)}
    >
      <div className="nav-session__top">
        <span className="nav-session__name">{session.graph_name ?? session.id}</span>
        <span className="nav-session__time">{session.started_at ? timeAgo(session.started_at) : ""}</span>
      </div>
      <div className="nav-session__bottom">
        <span className="nav-session__badge" style={statusBadgeStyle(color)}>
          {session.status === "running" && <span className="nav-session__pulse" />}
          {session.status}
        </span>
        {session.duration_ms != null && (
          <span className="nav-session__dur">{formatDuration(session.duration_ms)}</span>
        )}
        {session.node_count != null && (
          <span className="nav-session__nodes">{session.node_count}n</span>
        )}
      </div>
    </button>
  );
}
