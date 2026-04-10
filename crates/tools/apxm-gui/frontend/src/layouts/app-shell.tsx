import React from "react";
import { useAppStore } from "@/store/app-store";
import { ACTIVITIES } from "@/lib/constants";
import type { ActivityKey } from "@/lib/constants";
import { DashboardView } from "@/views/dashboard-view";
import { GraphView } from "@/views/graph-view";
import { SessionView } from "@/views/session-view";
import { LiveView } from "@/views/live-view";
import { ReplayView } from "@/views/replay-view";
import { OpsView } from "@/views/ops-view";
import { ConfigView } from "@/views/config-view";

const VIEW_MAP: Record<ActivityKey, React.ComponentType> = {
  dashboard: DashboardView,
  graph: GraphView,
  execution: SessionView,
  live: LiveView,
  replay: ReplayView,
  reference: OpsView,
  settings: ConfigView,
};

const ICONS: Record<ActivityKey, string> = {
  dashboard: "M4 5a1 1 0 011-1h4a1 1 0 011 1v4a1 1 0 01-1 1H5a1 1 0 01-1-1V5zm10 0a1 1 0 011-1h4a1 1 0 011 1v4a1 1 0 01-1 1h-4a1 1 0 01-1-1V5zM4 15a1 1 0 011-1h4a1 1 0 011 1v4a1 1 0 01-1 1H5a1 1 0 01-1-1v-4zm10 0a1 1 0 011-1h4a1 1 0 011 1v4a1 1 0 01-1 1h-4a1 1 0 01-1-1v-4z",
  graph: "M7 21l3-3m0 0l3 3m-3-3v8M4 4h16M4 4v16h16V4M9 9h6M9 9v6m6-6v6M9 15h6",
  execution: "M5 3l14 9-14 9V3z",
  live: "M12 12m-3 0a3 3 0 106 0 3 3 0 10-6 0 M12 12m-7 0a7 7 0 1014 0 7 7 0 10-14 0 M12 12m-11 0a11 11 0 1022 0 11 11 0 10-22 0",
  replay: "M3 12a9 9 0 109-9M3 3v9h9",
  reference: "M12 6.25a.75.75 0 01.75.75v3.25H16a.75.75 0 010 1.5h-3.25V15a.75.75 0 01-1.5 0v-3.25H8a.75.75 0 010-1.5h3.25V7a.75.75 0 01.75-.75z",
  settings: "M10.325 4.317c.426-1.756 2.924-1.756 3.35 0a1.724 1.724 0 002.573 1.066c1.543-.94 3.31.826 2.37 2.37a1.724 1.724 0 001.066 2.573c1.756.426 1.756 2.924 0 3.35a1.724 1.724 0 00-1.066 2.573c.94 1.543-.826 3.31-2.37 2.37a1.724 1.724 0 00-2.573 1.066c-.426 1.756-2.924 1.756-3.35 0a1.724 1.724 0 00-2.573-1.066c-1.543.94-3.31-.826-2.37-2.37a1.724 1.724 0 00-1.066-2.573c-1.756-.426-1.756-2.924 0-3.35a1.724 1.724 0 001.066-2.573c-.94-1.543.826-3.31 2.37-2.37.996.608 2.296.07 2.572-1.065z M15 12a3 3 0 11-6 0 3 3 0 016 0z",
};

export function AppShell() {
  const activeTab = useAppStore((s) => s.activeTab);
  const setActiveTab = useAppStore((s) => s.setActiveTab);
  const errors = useAppStore((s) => s.errors);

  const ActiveView = VIEW_MAP[activeTab as ActivityKey] ?? DashboardView;

  return (
    <div className="app-shell">
      <nav className="sidebar">
        <div className="sidebar__logo">
          <span className="sidebar__logo-text">AX</span>
        </div>
        <div className="sidebar__nav">
          {ACTIVITIES.map((activity) => (
            <button
              key={activity.key}
              type="button"
              className={`sidebar__item${activeTab === activity.key ? " sidebar__item--active" : ""}`}
              onClick={() => setActiveTab(activity.key)}
              title={`${activity.label} (${activity.shortcut})`}
            >
              <svg className="sidebar__icon" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.5">
                <path d={ICONS[activity.key]} />
              </svg>
              <span className="sidebar__label">{activity.label}</span>
            </button>
          ))}
        </div>
        <div className="sidebar__footer">
          <span className="sidebar__version">v0.1</span>
        </div>
      </nav>
      <div className="app-content">
        {errors.startup ? <div className="error-banner">{errors.startup}</div> : null}
        {errors.graph ? <div className="error-banner">{errors.graph}</div> : null}
        {errors.analysis ? <div className="error-banner">{errors.analysis}</div> : null}
        <main className="app-main">
          <ActiveView />
        </main>
      </div>
    </div>
  );
}
