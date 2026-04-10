import { useEffect, useState } from "react";
import type { ApxmGraph, GraphLayout } from "@/types/graph";
import { computeElkLayout } from "@/lib/graph-builder";

export function useElkLayout(
  graph: ApxmGraph | null,
  direction: "DOWN" | "RIGHT",
): { layout: GraphLayout | null; layoutError: string | null } {
  const [layout, setLayout] = useState<GraphLayout | null>(null);
  const [layoutError, setLayoutError] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;

    if (!graph) {
      setLayout(null);
      setLayoutError(null);
      return;
    }

    setLayout(null);
    setLayoutError(null);

    void computeElkLayout(graph, direction).then((nextLayout) => {
      if (cancelled) return;
      if (nextLayout) {
        setLayout(nextLayout);
      } else {
        setLayoutError("ELK layout computation failed");
      }
    });

    return () => { cancelled = true; };
  }, [graph, direction]);

  return { layout, layoutError };
}
