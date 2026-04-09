import { useEffect, useState } from "react";
import type { ApxmGraph, GraphLayout } from "@/types/graph";
import { computeElkLayout } from "@/lib/graph-builder";

export function useElkLayout(
  graph: ApxmGraph | null,
  direction: "DOWN" | "RIGHT",
): GraphLayout | null {
  const [layout, setLayout] = useState<GraphLayout | null>(null);

  useEffect(() => {
    let cancelled = false;

    if (!graph) {
      setLayout(null);
      return;
    }

    setLayout(null);

    void computeElkLayout(graph, direction).then((nextLayout) => {
      if (!cancelled) setLayout(nextLayout);
    });

    return () => { cancelled = true; };
  }, [graph, direction]);

  return layout;
}
