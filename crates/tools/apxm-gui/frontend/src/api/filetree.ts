import { apiFetch } from "./client";
import type { FileTreeResponse } from "@/types/api";

export async function fetchFileTree(): Promise<FileTreeResponse> {
  return apiFetch<FileTreeResponse>("/api/filetree");
}
