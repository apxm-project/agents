import React from "react";
import { useAppStore } from "@/store/app-store";
import { TABS } from "@/lib/constants";
import { GraphView } from "@/views/graph-view";
import { CompilerView } from "@/views/compiler-view";
import { SessionView } from "@/views/session-view";
import { LiveView } from "@/views/live-view";
import { OpsView } from "@/views/ops-view";
import { ConfigView } from "@/views/config-view";
import { ReplayView } from "@/views/replay-view";

const VIEW_MAP: Record<string, React.ComponentType> = {
  graph: GraphView,
  compiler: CompilerView,
  session: SessionView,
  live: LiveView,
  ops: OpsView,
  config: ConfigView,
  replay: ReplayView,
};

export function AppShell() {
  const activeTab = useAppStore((s) => s.activeTab);
  const setActiveTab = useAppStore((s) => s.setActiveTab);
  const errors = useAppStore((s) => s.errors);

  const ActiveView = VIEW_MAP[activeTab] ?? GraphView;

  return (
    <div className="app-shell">
      <nav className="tab-bar">
        {TABS.map((tab, i) => (
          <button
            key={tab.key}
            type="button"
            className={`tab-bar__tab${activeTab === tab.key ? " tab-bar__tab--active" : ""}`}
            onClick={() => setActiveTab(tab.key)}
          >
            <span className="tab-bar__key">{i + 1}</span>
            {tab.label}
          </button>
        ))}
      </nav>
      {errors.startup ? <div className="error-banner">{errors.startup}</div> : null}
      <main className="app-main">
        <ActiveView />
      </main>
    </div>
  );
}
