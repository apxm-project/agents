import { apiFetch } from "./client";
import type { WorkflowInfo } from "@/types/api";

export async function fetchWorkflows(): Promise<WorkflowInfo[]> {
  const data = await apiFetch<{ workflows: WorkflowInfo[] }>("/api/workflows");
  return data.workflows;
}
