import { apiFetch } from "./client";
import type { BackendDetail } from "@/types/api";

export async function fetchBackends(opts?: { probe?: boolean }): Promise<BackendDetail[]> {
  const params = opts?.probe ? "?probe=true" : "";
  const res = await apiFetch<{ backends: BackendDetail[] }>(`/api/backends${params}`);
  return res.backends;
}
