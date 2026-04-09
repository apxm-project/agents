import { apiFetch } from "./client";
import type { SessionData, SessionInfo } from "@/types/session";

export async function fetchSession(path: string): Promise<SessionData> {
  return apiFetch<SessionData>(`/api/session?path=${encodeURIComponent(path)}`);
}

export async function fetchSessionNode(nodeId: string, sessionPath: string): Promise<Record<string, unknown>> {
  return apiFetch<Record<string, unknown>>(`/api/session/node/${nodeId}?session=${encodeURIComponent(sessionPath)}`);
}

export async function fetchSessions(): Promise<SessionInfo[]> {
  return apiFetch<SessionInfo[]>("/api/sessions");
}
