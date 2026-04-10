import { apiFetch } from "./client";
import type { ApxmGraph } from "@/types/graph";
import type { GraphAnalysis, PassInfo } from "@/types/api";

export async function fetchGraph(path: string): Promise<ApxmGraph> {
  return apiFetch<ApxmGraph>(`/api/graph?path=${encodeURIComponent(path)}`);
}

export async function fetchGraphAnalysis(path: string): Promise<GraphAnalysis> {
  return apiFetch<GraphAnalysis>(`/api/graph/analyze?path=${encodeURIComponent(path)}`);
}

export async function fetchPasses(): Promise<PassInfo[]> {
  return apiFetch<PassInfo[]>("/api/passes");
}
