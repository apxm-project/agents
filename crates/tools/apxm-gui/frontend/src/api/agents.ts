import { apiFetch } from "./client";
import { parseSSEStream } from "./sse";
import * as EK from "@/lib/event-kinds";
import type { AgentProfile, AcpAgentProfile } from "@/types/api";
import type { AgentStreamEvent } from "@/types/events";

export async function fetchAgents(): Promise<AgentProfile[]> {
  const res = await apiFetch<{ agents: AgentProfile[] }>("/api/agents");
  return res.agents;
}

export async function fetchAgentProfiles(): Promise<AcpAgentProfile[]> {
  const res = await apiFetch<{ profiles: AcpAgentProfile[] }>("/api/agent/profiles");
  return res.profiles;
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
  agentId?: string,
): Promise<void> {
  const body: Record<string, unknown> = { message, session_id: sessionId };
  if (agentId) body.agent_id = agentId;

  const res = await fetch("/api/agent/chat", {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify(body),
    signal,
  });

  if (!res.ok) {
    const data = await res.json().catch(() => ({}));
    throw new Error(data.error ?? `Agent chat request failed: ${res.status}`);
  }

  await parseSSEStream(res, {
    onEvent: (eventType, parsed: any) => {
      const event = normalizeAgentStreamEvent(eventType, parsed);
      switch (event.kind) {
        case EK.TOKEN.name:
          if (event.token != null) callbacks.onToken(event.token);
          break;
        case EK.TOOL_CALL.name:
          callbacks.onToolCall({
            id: event.id,
            name: event.name,
            arguments: event.arguments ?? {},
          });
          break;
        case EK.TOOL_RESULT.name:
          callbacks.onToolResult({
            id: event.id,
            success: event.success,
            output: event.output ?? "",
          });
          break;
        case EK.USAGE.name:
          callbacks.onUsage({
            inputTokens: event.inputTokens ?? 0,
            outputTokens: event.outputTokens ?? 0,
          });
          break;
        case EK.DONE.name:
          callbacks.onDone(event.sessionId ?? "");
          break;
        case EK.ERROR.name:
          callbacks.onError(event.error ?? "Unknown agent error");
          break;
        default:
          if ("token" in event && event.token != null) callbacks.onToken(event.token);
          else if ("error" in event && event.error) callbacks.onError(event.error);
          break;
      }
    },
    onDone: () => callbacks.onDone(""),
  }, signal);
}

function normalizeAgentStreamEvent(eventType: string, parsed: any): AgentStreamEvent {
  const kind = asString(parsed?.kind, eventType);

  switch (kind) {
    case EK.TOKEN.name:
      return {
        kind: EK.TOKEN.name,
        token: asOptionalString(parsed?.token ?? parsed?.text),
      };
    case EK.TOOL_CALL.name:
      return {
        kind: EK.TOOL_CALL.name,
        id: asString(parsed?.id),
        name: asString(parsed?.name),
        arguments: asRecord(parsed?.arguments),
      };
    case EK.TOOL_RESULT.name:
      return {
        kind: EK.TOOL_RESULT.name,
        id: asString(parsed?.id),
        success: asBoolean(parsed?.success),
        output: asString(parsed?.output),
      };
    case EK.USAGE.name:
      return {
        kind: EK.USAGE.name,
        inputTokens: asNumber(parsed?.inputTokens ?? parsed?.input_tokens),
        outputTokens: asNumber(parsed?.outputTokens ?? parsed?.output_tokens),
      };
    case EK.DONE.name:
      return {
        kind: EK.DONE.name,
        sessionId: asString(parsed?.sessionId ?? parsed?.session_id),
      };
    case EK.ERROR.name:
      return {
        kind: EK.ERROR.name,
        error: asString(parsed?.error ?? parsed?.message, "Unknown agent error"),
      };
    default:
      return {
        kind: "unknown",
        rawKind: kind,
        token: asOptionalString(parsed?.token ?? parsed?.text),
        error: asOptionalString(parsed?.error ?? parsed?.message),
      };
  }
}

function asString(value: unknown, fallback = ""): string {
  return typeof value === "string" ? value : fallback;
}

function asOptionalString(value: unknown): string | undefined {
  return typeof value === "string" ? value : undefined;
}

function asNumber(value: unknown, fallback = 0): number {
  return typeof value === "number" ? value : fallback;
}

function asBoolean(value: unknown, fallback = false): boolean {
  return typeof value === "boolean" ? value : fallback;
}

function asRecord(value: unknown): Record<string, unknown> {
  return value && typeof value === "object" && !Array.isArray(value)
    ? (value as Record<string, unknown>)
    : {};
}
