import { apiFetch } from "./client";
import type { AgentProfile } from "@/types/api";

export async function fetchAgents(): Promise<AgentProfile[]> {
  const res = await apiFetch<{ agents: AgentProfile[] }>("/api/agents");
  return res.agents;
}
