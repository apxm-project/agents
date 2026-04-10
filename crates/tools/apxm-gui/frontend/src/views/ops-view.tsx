import { useState, useMemo } from "react";
import { useAppStore } from "@/store/app-store";
import { getCategoryStyle } from "@/lib/category-styles";
import { Section, CodeBlock } from "@/components/inspector/common";

export function OpsView() {
  const ops = useAppStore((s) => s.ops);
  const [search, setSearch] = useState("");
  const [selectedOp, setSelectedOp] = useState<string | null>(null);

  const filtered = useMemo(() => {
    if (!search) return ops;
    const q = search.toLowerCase();
    return ops.filter(
      (op) => op.name.toLowerCase().includes(q) || op.category.toLowerCase().includes(q) || op.description.toLowerCase().includes(q),
    );
  }, [ops, search]);

  const detail = useMemo(() => ops.find((op) => op.name === selectedOp) ?? null, [ops, selectedOp]);

  return (
    <div className="view-panel">
      <div className="view-panel__header">
        <h2>AIS Operations ({ops.length})</h2>
        <input
          type="text"
          className="search-input"
          placeholder="Search ops..."
          value={search}
          onChange={(e) => setSearch(e.target.value)}
        />
      </div>
      <div className="ops-layout">
        <div className="ops-table">
          {filtered.map((op) => {
            const style = getCategoryStyle(op.category);
            return (
              <button
                key={op.name}
                type="button"
                className={`ops-table__row${selectedOp === op.name ? " ops-table__row--selected" : ""}`}
                onClick={() => setSelectedOp(op.name)}
              >
                <span className="ops-table__category" style={{ background: style.bg, color: style.color }}>{op.category}</span>
                <span className="ops-table__name">{op.name}</span>
                <span className="ops-table__latency">{op.latency}</span>
              </button>
            );
          })}
        </div>
        <div className="ops-detail">
          {detail ? (
            <div className="ops-detail__content">
              <Section title={detail.name} subtitle={`${detail.category} \u00B7 ${detail.latency} latency`}>
                <p className="inspector__description">{detail.description}</p>
                {detail.long_description ? <p className="inspector__description">{detail.long_description}</p> : null}
              </Section>
              {detail.fields.length > 0 ? (
                <Section title="Fields">
                  <dl className="definition-grid">
                    {detail.fields.map((f) => (
                      <div key={f.name}>
                        <dt>{f.name}{f.required ? " *" : ""}</dt>
                        <dd>{f.description}</dd>
                      </div>
                    ))}
                  </dl>
                </Section>
              ) : null}
              {detail.example_json ? (
                <Section title="Example">
                  <CodeBlock>{detail.example_json}</CodeBlock>
                </Section>
              ) : null}
            </div>
          ) : (
            <div className="view-panel__empty">Select an operation to see details.</div>
          )}
        </div>
      </div>
    </div>
  );
}
