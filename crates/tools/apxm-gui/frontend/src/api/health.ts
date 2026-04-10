import { apiFetch } from "./client";
import type { HealthStatus } from "@/types/api";

export async function fetchHealth(opts?: { probe?: boolean }): Promise<HealthStatus> {
  const params = opts?.probe ? "?probe=true" : "";
  return apiFetch<HealthStatus>(`/api/health${params}`);
}
