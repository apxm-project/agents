import { apiFetch } from "./client";
import type { StartupInfo } from "@/types/api";

export async function fetchStartup(): Promise<StartupInfo> {
  return apiFetch<StartupInfo>("/api/startup");
}
