import { apiFetch } from "./client";
import type { ApxmGraph } from "@/types/graph";

export async function saveGraph(
  graph: ApxmGraph,
  path?: string,
): Promise<{ path: string }> {
  return apiFetch<{ path: string }>("/api/graph/save", {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ graph, path: path ?? null }),
  });
}
