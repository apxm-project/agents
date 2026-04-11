import { apiFetch } from "./client";
import type { SkillInfo, SkillDetail } from "@/types/api";

export async function fetchSkills(): Promise<SkillInfo[]> {
  const res = await apiFetch<{ skills: SkillInfo[] }>("/api/skills");
  return res.skills;
}

export async function fetchSkillDetail(name: string): Promise<SkillDetail> {
  return apiFetch<SkillDetail>(`/api/skills/${encodeURIComponent(name)}`);
}
