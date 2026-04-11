import { apiFetch } from "./client";
import { parseSSEStream } from "./sse";
import { SseEvent } from "@/lib/constants";
import type { AgentProfile } from "@/types/api";

export async function fetchAgents(): Promise<AgentProfile[]> {
  const res = await apiFetch<{ agents: AgentProfile[] }>("/api/agents");
  return res.agents;
}

export type ToolCallEvent = {
  id: string;
  name: string;
  arguments: Record<string, unknown>;
};

export type ToolResultEvent = {
  id: string;
  success: boolean;
  output: string;
};

export type UsageEvent = {
  inputTokens: number;
  outputTokens: number;
};

export type AgentCallbacks = {
  onToken: (token: string) => void;
  onToolCall: (call: ToolCallEvent) => void;
  onToolResult: (result: ToolResultEvent) => void;
  onUsage: (usage: UsageEvent) => void;
  onDone: (sessionId: string) => void;
  onError: (error: string) => void;
};

export async function streamAgentChat(
  message: string,
  sessionId: string | null,
  callbacks: AgentCallbacks,
  signal?: AbortSignal,
): Promise<void> {
  const res = await fetch("/api/agent/chat", {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ message, session_id: sessionId }),
    signal,
  });

  if (!res.ok) {
    const data = await res.json().catch(() => ({}));
    throw new Error(data.error ?? `Agent chat request failed: ${res.status}`);
  }

  await parseSSEStream(res, {
    onEvent: (eventType, parsed: any) => {
      switch (eventType) {
        case SseEvent.TOKEN:
          if (parsed.token != null) callbacks.onToken(parsed.token);
          break;
        case SseEvent.TOOL_CALL:
          callbacks.onToolCall({
            id: parsed.id,
            name: parsed.name,
            arguments: parsed.arguments ?? {},
          });
          break;
        case SseEvent.TOOL_RESULT:
          callbacks.onToolResult({
            id: parsed.id,
            success: parsed.success,
            output: parsed.output ?? "",
          });
          break;
        case SseEvent.USAGE:
          callbacks.onUsage({
            inputTokens: parsed.inputTokens ?? 0,
            outputTokens: parsed.outputTokens ?? 0,
          });
          break;
        case SseEvent.DONE:
          callbacks.onDone(parsed.sessionId ?? "");
          break;
        case SseEvent.ERROR:
          callbacks.onError(parsed.error ?? "Unknown agent error");
          break;
        default:
          if (parsed.token != null) callbacks.onToken(parsed.token);
          else if (parsed.error) callbacks.onError(parsed.error);
          break;
      }
    },
    onDone: () => callbacks.onDone(""),
  }, signal);
}
