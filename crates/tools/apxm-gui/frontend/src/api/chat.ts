import { SseEvent } from "@/lib/constants";
import { parseSSEStream } from "./sse";
import type { ChatRole } from "@/lib/constants";

const BASE = "";

export type ChatMessage = {
  role: ChatRole;
  content: string;
};

export type ModelInfo = {
  id: string;
  aliases: string[];
};

export async function fetchChatModels(): Promise<ModelInfo[]> {
  const res = await fetch(`${BASE}/api/chat/models`);
  if (!res.ok) throw new Error(`Failed to fetch models: ${res.status}`);
  const data = await res.json();
  return data.models ?? [];
}

export async function streamChat(
  messages: ChatMessage[],
  model: string,
  onToken: (token: string) => void,
  onDone: () => void,
  onError: (error: string) => void,
  signal?: AbortSignal,
): Promise<void> {
  const res = await fetch(`${BASE}/api/chat`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ messages, model, stream: true }),
    signal,
  });

  if (!res.ok) {
    const data = await res.json().catch(() => ({}));
    throw new Error(data.error ?? `Chat request failed: ${res.status}`);
  }

  await parseSSEStream(res, {
    onEvent: (eventType, parsed: any) => {
      if (eventType === SseEvent.TOKEN && parsed.token) {
        onToken(parsed.token);
      } else if (eventType === SseEvent.DONE || parsed.finish_reason) {
        onDone();
      } else if (eventType === SseEvent.ERROR || parsed.error) {
        onError(parsed.error ?? "Unknown error");
      } else if (parsed.token) {
        onToken(parsed.token);
      }
    },
    onDone,
  }, signal);
}
