import { useEffect, useRef } from "react";

type SSEOptions = {
  url: string | null;
  onEvent: (eventType: string, data: string) => void;
  onError?: (error: Event) => void;
  onOpen?: () => void;
  reconnectMs?: number;
};

export function useSSE({ url, onEvent, onError, onOpen, reconnectMs = 3000 }: SSEOptions) {
  const reconnectTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  useEffect(() => {
    if (!url) return;

    let source: EventSource | null = null;
    let cancelled = false;

    function connect() {
      if (cancelled || !url) return;
      source = new EventSource(url);

      source.onopen = () => {
        if (!cancelled) onOpen?.();
      };

      source.onerror = (e) => {
        if (cancelled) return;
        onError?.(e);
        source?.close();
        reconnectTimerRef.current = setTimeout(connect, reconnectMs);
      };

      // Listen for named events
      for (const eventType of ["trace", "progress", "status", "output"]) {
        source.addEventListener(eventType, (e) => {
          if (!cancelled) onEvent(eventType, (e as MessageEvent).data);
        });
      }
    }

    connect();

    return () => {
      cancelled = true;
      source?.close();
      if (reconnectTimerRef.current) {
        clearTimeout(reconnectTimerRef.current);
        reconnectTimerRef.current = null;
      }
    };
  }, [url, onEvent, onError, onOpen, reconnectMs]);
}
