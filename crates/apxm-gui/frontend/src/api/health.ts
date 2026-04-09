import { apiFetch } from "./client";
import type { HealthStatus } from "@/types/api";

export async function fetchHealth(): Promise<HealthStatus> {
  return apiFetch<HealthStatus>("/api/health");
}
