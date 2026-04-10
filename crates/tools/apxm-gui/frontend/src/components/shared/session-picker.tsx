import type { SessionInfo } from "@/types/session";
import { formatAbsoluteTimestamp, STATUS_COLORS, FALLBACK_STATUS_COLOR, statusBadgeStyle, sessionPath } from "@/lib/format";

type SessionPickerProps = {
  sessions: SessionInfo[];
  title: string;
  emptyMessage: string;
  actionLabel: string;
  actionVariant?: "live" | "default";
  onSelect: (path: string) => void;
};

export function SessionPicker({ sessions, title, emptyMessage, actionLabel, actionVariant, onSelect }: SessionPickerProps) {
  return (
    <div className="session-picker">
      {sessions.length === 0 ? (
        <div className="view-panel__empty">{emptyMessage}</div>
      ) : (
        <>
          <div className="session-picker__title">{title}</div>
          {sessions.map((s) => {
            const color = STATUS_COLORS[s.status] ?? FALLBACK_STATUS_COLOR;
            return (
              <div key={s.id} className="session-picker__item">
                <div className="session-picker__info">
                  <span className="session-picker__name">{s.graph_name ?? s.id}</span>
                  <span className="session-picker__meta">
                    <span style={statusBadgeStyle(color)}>{s.status}</span>
                    <span>{formatAbsoluteTimestamp(s.started_at)}</span>
                  </span>
                </div>
                <button
                  type="button"
                  className={`session-action-btn${actionVariant === "live" ? " session-action-btn--live" : ""}`}
                  onClick={() => onSelect(sessionPath(s.id))}
                >
                  {actionLabel}
                </button>
              </div>
            );
          })}
        </>
      )}
    </div>
  );
}
