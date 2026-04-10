import { useState, useCallback, useEffect } from "react";
import { useAppStore } from "@/store/app-store";
import { fetchSession, fetchSessions } from "@/api/session";
import { getErrorMessage } from "@/lib/format";
import { SessionPicker } from "@/components/shared/session-picker";
import { EventFeed } from "@/components/shared/event-feed";
import type { TraceEvent } from "@/types/events";

export function ReplayView() {
  const [trace, setTrace] = useState<TraceEvent[]>([]);
  const [cursor, setCursor] = useState(0);
  const [error, setError] = useState<string | null>(null);
  const sessions = useAppStore((s) => s.sessions);
  const setSessions = useAppStore((s) => s.setSessions);
  const replaySessionPath = useAppStore((s) => s.replaySessionPath);

  const loadSession = useCallback(async (path: string) => {
    try {
      const data = await fetchSession(path);
      setTrace(data.trace);
      setCursor(0);
      setError(null);
    } catch (e) {
      setError(getErrorMessage(e));
    }
  }, []);

  useEffect(() => {
    if (sessions.length === 0) {
      fetchSessions().then(setSessions).catch(() => {});
    }
  }, [sessions.length, setSessions]);

  useEffect(() => {
    if (replaySessionPath && trace.length === 0) {
      loadSession(replaySessionPath);
    }
  }, [replaySessionPath, trace.length, loadSession]);

  const handleStep = useCallback(() => {
    setCursor((c) => Math.min(c + 1, trace.length));
  }, [trace.length]);

  const visibleEvents = trace.slice(0, cursor);
  const completedSessions = sessions.filter((s) => s.status === "completed" || s.status === "failed");

  if (trace.length === 0) {
    return (
      <div className="view-panel">
        <div className="view-panel__header">
          <h2>Replay</h2>
        </div>
        {error ? <div className="error-banner">{error}</div> : null}
        <SessionPicker
          sessions={completedSessions}
          title="Select a Session to Replay"
          emptyMessage="No completed sessions to replay."
          actionLabel="Replay"
          onSelect={loadSession}
        />
      </div>
    );
  }

  return (
    <div className="view-panel">
      <div className="view-panel__header">
        <h2>Replay</h2>
        <div className="view-panel__actions">
          <button type="button" className="ghost-button" onClick={() => { setTrace([]); setCursor(0); }}>Back</button>
          <button type="button" className="ghost-button" onClick={() => setCursor(0)}>Reset</button>
          <button type="button" className="ghost-button" onClick={handleStep} disabled={cursor >= trace.length}>Step</button>
          <button type="button" className="ghost-button" onClick={() => setCursor(trace.length)}>End</button>
        </div>
      </div>
      {error ? <div className="error-banner">{error}</div> : null}
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
        <EventFeed events={visibleEvents} />
      </div>
    </div>
  );
}
