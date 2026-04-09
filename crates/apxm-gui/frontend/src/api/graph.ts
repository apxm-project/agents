import { apiFetch } from "./client";
import type { ApxmGraph } from "@/types/graph";
import type { GraphAnalysis, OptimizedResult } from "@/types/api";

export async function fetchGraph(path: string): Promise<ApxmGraph> {
  return apiFetch<ApxmGraph>(`/api/graph?path=${encodeURIComponent(path)}`);
}

export async function fetchGraphAnalysis(path: string): Promise<GraphAnalysis> {
  return apiFetch<GraphAnalysis>(`/api/graph/analyze?path=${encodeURIComponent(path)}`);
}

export async function fetchOptimized(path: string, passes?: string[]): Promise<OptimizedResult> {
  let url = `/api/graph/optimized?path=${encodeURIComponent(path)}`;
  if (passes && passes.length > 0) {
    url += `&passes=${encodeURIComponent(passes.join(","))}`;
  }
  return apiFetch<OptimizedResult>(url);
}
