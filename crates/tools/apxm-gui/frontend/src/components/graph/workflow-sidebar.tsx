import { useCallback, useEffect, useMemo, useState } from "react";
import { useAppStore } from "@/store/app-store";
import { fetchWorkflows, fetchFile } from "@/api";
import { ViewMode, LoadKey } from "@/lib/constants";
import type { WorkflowInfo } from "@/types/api";

const SOURCE_BADGE_COLORS: Record<string, string> = {
  py: "rgba(78, 154, 218, 0.18)",
  air: "rgba(88, 187, 129, 0.18)",
  apxm: "rgba(212, 155, 69, 0.18)",
};
const SOURCE_TEXT_COLORS: Record<string, string> = {
  py: "#5eaadd",
  air: "#58bb81",
  apxm: "#d49b45",
};

export function WorkflowSidebar() {
  const workflows = useAppStore((s) => s.workflows);
  const setWorkflows = useAppStore((s) => s.setWorkflows);
  const selectedWorkflow = useAppStore((s) => s.selectedWorkflow);
  const setSelectedWorkflow = useAppStore((s) => s.setSelectedWorkflow);
  const setSourceContent = useAppStore((s) => s.setSourceContent);
  const setViewMode = useAppStore((s) => s.setViewMode);
  const setLoading = useAppStore((s) => s.setLoading);
  const setError = useAppStore((s) => s.setError);

  const [filter, setFilter] = useState("");
  const [refreshKey, setRefreshKey] = useState(0);

  useEffect(() => {
    let cancelled = false;
    setLoading(LoadKey.WORKFLOWS, true);
    fetchWorkflows()
      .then((w) => {
        if (!cancelled) {
          setWorkflows(w);
          setError(LoadKey.WORKFLOWS, null);
        }
      })
      .catch((e) => {
        if (!cancelled) setError(LoadKey.WORKFLOWS, e.message);
      })
      .finally(() => {
        if (!cancelled) setLoading(LoadKey.WORKFLOWS, false);
      });
    return () => { cancelled = true; };
  }, [refreshKey, setWorkflows, setLoading, setError]);

  const grouped = useMemo(() => {
    const q = filter.toLowerCase();
    const filtered = workflows.filter(
      (w) =>
        w.name.toLowerCase().includes(q) ||
        w.category.toLowerCase().includes(q) ||
        w.relative_path.toLowerCase().includes(q),
    );
    const map = new Map<string, WorkflowInfo[]>();
    for (const w of filtered) {
      const cat = w.category || "root";
      const list = map.get(cat) ?? [];
      list.push(w);
      map.set(cat, list);
    }
    return Array.from(map.entries()).sort(([a], [b]) => a.localeCompare(b));
  }, [workflows, filter]);

  const totalCount = useMemo(
    () => grouped.reduce((n, [, list]) => n + list.length, 0),
    [grouped],
  );

  const onSelect = useCallback(
    async (w: WorkflowInfo) => {
      setSelectedWorkflow(w);
      setViewMode(ViewMode.SOURCE);
      setLoading(LoadKey.SOURCE, true);
      try {
        const content = await fetchFile(w.path);
        setSourceContent(content, w.path);
        setError(LoadKey.SOURCE, null);
      } catch (e: unknown) {
        setError(LoadKey.SOURCE, e instanceof Error ? e.message : String(e));
        setSourceContent(null);
      } finally {
        setLoading(LoadKey.SOURCE, false);
      }
    },
    [setSelectedWorkflow, setViewMode, setSourceContent, setLoading, setError],
  );

  return (
    <div className="fside">
      <div className="fside__header">
        <span className="fside__title">Workflows</span>
        <span className="fside__count">{totalCount}</span>
        <button
          type="button"
          className="fside__refresh"
          onClick={() => setRefreshKey((k) => k + 1)}
          title="Refresh workflows"
        >
          ↻
        </button>
      </div>

      <div className="fside__search">
        <input
          className="fside__search-input"
          type="text"
          placeholder="Filter workflows…"
          value={filter}
          onChange={(e) => setFilter(e.target.value)}
        />
      </div>

      <div className="fside__tree">
        {grouped.length === 0 ? (
          <div className="fside__empty">
            No workflows found.{"\n"}Place .py or .air files in the project.
          </div>
        ) : (
          grouped.map(([category, items]) => (
            <div key={category} className="wf-category">
              <div className="wf-category__label">{category}</div>
              {items.map((w) => {
                const isActive =
                  selectedWorkflow?.path === w.path;
                return (
                  <button
                    key={w.path}
                    type="button"
                    className={`wf-entry${isActive ? " wf-entry--active" : ""}`}
                    onClick={() => onSelect(w)}
                    title={[w.description, w.parameters?.length ? `params: ${w.parameters.map((p) => p.name).join(", ")}` : ""].filter(Boolean).join("\n") || undefined}
                  >
                    <div className="wf-entry__top">
                      <span className="wf-entry__name">{w.name}</span>
                      <span className="wf-entry__meta">
                        <span
                          className="wf-entry__badge"
                          style={{
                            background: SOURCE_BADGE_COLORS[w.source_type] ?? "rgba(255,255,255,0.06)",
                            color: SOURCE_TEXT_COLORS[w.source_type] ?? "var(--ink-faint)",
                          }}
                        >
                          {w.source_type}
                        </span>
                        {w.node_count != null && (
                          <span className="wf-entry__nodes">{w.node_count}n</span>
                        )}
                      </span>
                    </div>
                  </button>
                );
              })}
            </div>
          ))
        )}
      </div>
    </div>
  );
}
