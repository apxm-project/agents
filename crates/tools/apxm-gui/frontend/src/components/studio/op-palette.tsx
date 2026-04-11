import { useMemo, useState } from "react";
import { useAppStore } from "@/store/app-store";
import { getCategoryStyle } from "@/lib/category-styles";

export function OpPalette() {
  const ops = useAppStore((s) => s.ops);
  const addNode = useAppStore((s) => s.addNode);
  const [filter, setFilter] = useState("");
  const [expandedCat, setExpandedCat] = useState<Set<string>>(new Set());

  const grouped = useMemo(() => {
    const q = filter.toLowerCase();
    const filtered = ops.filter(
      (op) =>
        op.name.toLowerCase().includes(q) ||
        op.category.toLowerCase().includes(q) ||
        op.description.toLowerCase().includes(q),
    );
    const map = new Map<string, typeof ops>();
    for (const op of filtered) {
      const list = map.get(op.category) ?? [];
      list.push(op);
      map.set(op.category, list);
    }
    return Array.from(map.entries()).sort(([a], [b]) => a.localeCompare(b));
  }, [ops, filter]);

  function toggleCat(cat: string) {
    setExpandedCat((prev) => {
      const next = new Set(prev);
      if (next.has(cat)) next.delete(cat); else next.add(cat);
      return next;
    });
  }

  function handleDragStart(e: React.DragEvent, op: typeof ops[0]) {
    e.dataTransfer.setData("application/apxm-op", JSON.stringify({
      name: op.name,
      op_type: op.op_type,
      category: op.category,
    }));
    e.dataTransfer.effectAllowed = "copy";
  }

  function handleClick(op: typeof ops[0]) {
    addNode(op.name, op.name.toLowerCase().replace(/\s+/g, "_"));
  }

  return (
    <div className="op-palette">
      <div className="op-palette__header">
        <span className="op-palette__title">Operations</span>
        <span className="op-palette__count">{ops.length}</span>
      </div>
      <input
        className="op-palette__search"
        type="text"
        placeholder="Filter operations..."
        value={filter}
        onChange={(e) => setFilter(e.target.value)}
      />
      <div className="op-palette__list">
        {grouped.map(([category, items]) => {
          const style = getCategoryStyle(category);
          const isExpanded = expandedCat.has(category) || filter.length > 0;
          return (
            <div key={category} className="op-palette__category">
              <button
                type="button"
                className="op-palette__cat-header"
                onClick={() => toggleCat(category)}
              >
                <span className="op-palette__cat-dot" style={{ background: style.color }} />
                <span className="op-palette__cat-name">{category}</span>
                <span className="op-palette__cat-count">{items.length}</span>
                <span className="op-palette__cat-chevron">{isExpanded ? "▾" : "▸"}</span>
              </button>
              {isExpanded && (
                <div className="op-palette__ops">
                  {items.map((op) => (
                    <div
                      key={op.name}
                      className="op-palette__op"
                      draggable
                      onDragStart={(e) => handleDragStart(e, op)}
                      onClick={() => handleClick(op)}
                      title={op.description}
                    >
                      <span className="op-palette__op-name">{op.name}</span>
                      <span className="op-palette__op-latency">{op.latency}</span>
                    </div>
                  ))}
                </div>
              )}
            </div>
          );
        })}
      </div>
    </div>
  );
}
