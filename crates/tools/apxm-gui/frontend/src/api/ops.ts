import { apiFetch } from "./client";
import type { OpSpec } from "@/types/ops";

export async function fetchOps(): Promise<OpSpec[]> {
  return apiFetch<OpSpec[]>("/api/ops");
}
