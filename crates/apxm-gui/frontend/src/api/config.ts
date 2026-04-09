import { apiFetch } from "./client";

export async function fetchConfig(): Promise<string> {
  return apiFetch<string>("/api/config");
}
