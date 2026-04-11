import { apiFetch } from "./client";

export async function fetchFile(path: string): Promise<string> {
  return apiFetch<string>(`/api/file?path=${encodeURIComponent(path)}`);
}
