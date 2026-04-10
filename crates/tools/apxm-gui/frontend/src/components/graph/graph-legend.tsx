import { useState } from "react";
import { getCategoryStyle, getEdgeStyle } from "@/lib/category-styles";

type GraphLegendProps = { categories: string[] };

export function GraphLegend({ categories }: GraphLegendProps) {
  const [collapsed, setCollapsed] = useState(false);
  if (categories.length === 0) return null;

  return (
    <div className="graph-legend">
      <button type="button" className="graph-legend__toggle" onClick={() => setCollapsed((c) => !c)}>
        {collapsed ? "Legend \u25B6" : "Legend \u25BC"}
      </button>
      {!collapsed ? (
        <div className="graph-legend__body">
          <div className="graph-legend__section">
            <div className="graph-legend__section-title">Categories</div>
            {categories.map((cat) => {
              const style = getCategoryStyle(cat);
              return (
                <div key={cat} className="graph-legend__item">
                  <span className="graph-legend__swatch" style={{ background: style.color }} />
                  <span className="graph-legend__label">{cat}</span>
                </div>
              );
            })}
          </div>
          <div className="graph-legend__section">
            <div className="graph-legend__section-title">Edge Types</div>
            {(["Data", "Control", "Effect"] as const).map((dep) => {
              const style = getEdgeStyle(dep);
              return (
                <div key={dep} className="graph-legend__item">
                  <svg width="24" height="10" className="graph-legend__edge-sample">
                    <line x1="0" y1="5" x2="24" y2="5" stroke={style.stroke} strokeWidth="2" strokeDasharray={style.strokeDasharray} />
                  </svg>
                  <span className="graph-legend__label">{dep}</span>
                </div>
              );
            })}
          </div>
        </div>
      ) : null}
    </div>
  );
}
