const BASE = "";

export type ChatMessage = {
  role: "user" | "assistant" | "system";
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

  const reader = res.body?.getReader();
  if (!reader) {
    throw new Error("No response body");
  }

  const decoder = new TextDecoder();
  let buffer = "";
  let currentEvent = "";

  while (true) {
    const { done, value } = await reader.read();
    if (done) break;

    buffer += decoder.decode(value, { stream: true });
    const lines = buffer.split("\n");
    buffer = lines.pop() ?? "";

    for (const line of lines) {
      const trimmed = line.trim();
      if (trimmed.startsWith(":")) continue;

      // Empty line = end of SSE event block
      if (!trimmed) {
        currentEvent = "";
        continue;
      }

      if (trimmed.startsWith("event: ")) {
        currentEvent = trimmed.slice(7);
        continue;
      }

      if (trimmed.startsWith("data: ")) {
        const dataStr = trimmed.slice(6);
        if (dataStr === "[DONE]") {
          onDone();
          continue;
        }
        try {
          const parsed = JSON.parse(dataStr);
          if (currentEvent === "token" && parsed.token) {
            onToken(parsed.token);
          } else if (currentEvent === "done" || parsed.finish_reason) {
            onDone();
          } else if (currentEvent === "error" || parsed.error) {
            onError(parsed.error ?? "Unknown error");
          } else if (parsed.token) {
            onToken(parsed.token);
          }
        } catch {
          // Skip unparseable lines
        }
      }
    }
  }
}
