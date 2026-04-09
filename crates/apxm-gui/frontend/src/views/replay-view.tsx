import { useState, useCallback } from "react";
import { fetchSession } from "@/api/session";
import { formatPreciseTimestamp, getErrorMessage } from "@/lib/format";
import { eventSummary } from "@/lib/event-utils";
import type { TraceEvent } from "@/types/events";

export function ReplayView() {
  const [trace, setTrace] = useState<TraceEvent[]>([]);
  const [cursor, setCursor] = useState(0);
  const [error, setError] = useState<string | null>(null);

  const handleLoad = useCallback(async () => {
    const path = prompt("Enter session path:");
    if (!path) return;
    try {
      const data = await fetchSession(path);
      setTrace(data.trace);
      setCursor(0);
    } catch (e) {
      setError(getErrorMessage(e));
    }
  }, []);

  const handleStep = useCallback(() => {
    setCursor((c) => Math.min(c + 1, trace.length));
  }, [trace.length]);

  const visibleEvents = trace.slice(0, cursor);

  return (
    <div className="view-panel">
      <div className="view-panel__header">
        <h2>Replay</h2>
        <div className="view-panel__actions">
          <button type="button" className="ghost-button" onClick={handleLoad}>Load Session</button>
          {trace.length > 0 ? (
            <>
              <button type="button" className="ghost-button" onClick={() => setCursor(0)}>Reset</button>
              <button type="button" className="ghost-button" onClick={handleStep} disabled={cursor >= trace.length}>Step</button>
              <button type="button" className="ghost-button" onClick={() => setCursor(trace.length)}>End</button>
            </>
          ) : null}
        </div>
      </div>
      {error ? <div className="error-banner">{error}</div> : null}
      {trace.length > 0 ? (
        <div className="replay-layout">
          <div className="replay-timeline">
            <input
              type="range"
              min={0}
              max={trace.length}
              value={cursor}
              onChange={(e) => setCursor(Number(e.target.value))}
              className="replay-slider"
            />
            <span className="replay-counter">{cursor} / {trace.length}</span>
          </div>
          <div className="event-feed">
            {visibleEvents.slice(-100).reverse().map((event, i) => (
              <div key={i} className="event-feed__item">
                <span className="event-feed__time">{formatPreciseTimestamp(event.timestamp)}</span>
                <span className="event-feed__kind">{event.kind}</span>
                <span className="event-feed__summary">{eventSummary(event)}</span>
              </div>
            ))}
          </div>
        </div>
      ) : (
        <div className="view-panel__empty">Load a session to replay its execution trace.</div>
      )}
    </div>
  );
}
