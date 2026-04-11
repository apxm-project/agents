import { apiFetch } from "./client";
import type { CompileResult, ExecuteResult, ValidateResult, DecompileResult, ExplainResult } from "@/types/api";

export type CompileOptions = {
  opt_level?: number;
  target?: string;
  emit_diagnostics?: boolean;
  no_cse_llm?: boolean;
};

export async function compileWorkflow(
  path: string,
  opts: CompileOptions = {},
): Promise<CompileResult> {
  return apiFetch<CompileResult>("/api/compile", {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({
      path,
      opt_level: opts.opt_level ?? 1,
      target: opts.target,
      emit_diagnostics: opts.emit_diagnostics ?? false,
      no_cse_llm: opts.no_cse_llm ?? false,
    }),
  });
}

export async function executeWorkflow(
  path: string,
  params: Record<string, string> = {},
  optLevel = 1,
): Promise<ExecuteResult> {
  return apiFetch<ExecuteResult>("/api/execute", {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ path, params, opt_level: optLevel }),
  });
}

export async function validateWorkflow(path: string): Promise<ValidateResult> {
  return apiFetch<ValidateResult>("/api/validate", {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ path }),
  });
}

export async function decompileArtifact(path: string): Promise<DecompileResult> {
  return apiFetch<DecompileResult>("/api/decompile", {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ path }),
  });
}

export async function explainWorkflow(path: string): Promise<ExplainResult> {
  return apiFetch<ExplainResult>(`/api/explain?path=${encodeURIComponent(path)}`);
}

export async function updateConfig(content: string): Promise<void> {
  await apiFetch("/api/config/update", {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ content }),
  });
}
