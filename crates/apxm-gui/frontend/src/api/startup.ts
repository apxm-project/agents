import { apiFetch } from "./client";
import type { StartupInfo, ExampleInfo } from "@/types/api";

export async function fetchStartup(): Promise<StartupInfo> {
  return apiFetch<StartupInfo>("/api/startup");
}

export async function fetchExamples(): Promise<ExampleInfo[]> {
  const data = await apiFetch<{ examples: ExampleInfo[] }>("/api/examples");
  return data.examples;
}
