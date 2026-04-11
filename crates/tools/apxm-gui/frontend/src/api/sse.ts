/// Shared SSE (Server-Sent Events) stream parser for fetch-based streaming.

export type SSEHandler = {
  onEvent: (eventType: string, data: unknown) => void;
  onDone?: () => void;
};

/**
 * Parse a fetch Response body as an SSE stream, dispatching typed events.
 *
 * Handles the standard SSE wire format: `event:` lines, `data:` lines,
 * comment lines (`:`), and blank-line delimiters. JSON-parses each `data:`
 * payload before dispatching.
 */
export async function parseSSEStream(
  response: Response,
  handler: SSEHandler,
  signal?: AbortSignal,
): Promise<void> {
  const reader = response.body?.getReader();
  if (!reader) {
    throw new Error("No response body");
  }

  const decoder = new TextDecoder();
  let buffer = "";
  let currentEvent = "";

  try {
    while (true) {
      if (signal?.aborted) break;

      const { done, value } = await reader.read();
      if (done) break;

      buffer += decoder.decode(value, { stream: true });
      const lines = buffer.split("\n");
      buffer = lines.pop() ?? "";

      for (const line of lines) {
        const trimmed = line.trim();

        // Comment line
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
            handler.onDone?.();
            continue;
          }
          try {
            const parsed = JSON.parse(dataStr);
            handler.onEvent(currentEvent, parsed);
          } catch {
            // Skip unparseable data lines
          }
        }
      }
    }
  } finally {
    reader.releaseLock();
  }
}
