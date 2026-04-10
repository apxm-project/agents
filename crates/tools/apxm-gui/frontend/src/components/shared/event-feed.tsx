import { formatPreciseTimestamp } from "@/lib/format";
import { eventSummary } from "@/lib/event-utils";
import type { TraceEvent } from "@/types/events";
import type React from "react";

type EventFeedProps = {
  events: TraceEvent[];
  limit?: number;
  style?: React.CSSProperties;
};

export function EventFeed({ events, limit = 100, style }: EventFeedProps) {
  return (
    <div className="event-feed" style={style}>
      {events.slice(-limit).reverse().map((event, i) => (
        <div key={i} className="event-feed__item">
          <span className="event-feed__time">{formatPreciseTimestamp(event.timestamp)}</span>
          <span className="event-feed__kind">{event.kind}</span>
          <span className="event-feed__summary">{eventSummary(event)}</span>
        </div>
      ))}
    </div>
  );
}
