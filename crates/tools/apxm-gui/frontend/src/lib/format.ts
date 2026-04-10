import type React from "react";

export function truncate(str: string, maxLen: number): string {
  if (str.length <= maxLen) return str;
  return str.slice(0, maxLen - 1) + "\u2026";
}

export function formatDuration(ms: number): string {
  if (ms < 1000) return `${Math.round(ms)}ms`;
  if (ms < 60_000) return `${(ms / 1000).toFixed(1)}s`;
  const mins = Math.floor(ms / 60_000);
  const secs = ((ms % 60_000) / 1000).toFixed(0);
  return `${mins}m ${secs}s`;
}

export function formatAbsoluteTimestamp(iso: string): string {
  const d = new Date(iso);
  if (isNaN(d.getTime())) return iso;
  const pad = (n: number) => String(n).padStart(2, "0");
  return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())} ${pad(d.getHours())}:${pad(d.getMinutes())}`;
}

export function formatPreciseTimestamp(iso: string): string {
  const d = new Date(iso);
  if (isNaN(d.getTime())) return iso;
  const pad = (n: number) => String(n).padStart(2, "0");
  return `${pad(d.getHours())}:${pad(d.getMinutes())}:${pad(d.getSeconds())}.${String(d.getMilliseconds()).padStart(3, "0")}`;
}

export function humanizeNodeName(name: string): string {
  return name.replace(/_/g, " ").replace(/\b\w/g, (c) => c.toUpperCase());
}

export function getErrorMessage(e: unknown): string {
  return e instanceof Error ? e.message : String(e);
}

export function stripNodePrefix(key: string): string {
  return key.replace(/^\d+_?/, "");
}

export const FALLBACK_STATUS_COLOR = "#8b949e";

export function statusBadgeStyle(color: string): React.CSSProperties {
  return { background: `${color}1a`, color };
}

export const STATUS_COLORS: Record<string, string> = {
  pending: "#8b949e",
  running: "#3ea6c6",
  completed: "#58bb81",
  failed: "#d86b6b",
  healthy: "#58bb81",
  degraded: "#d49b45",
  unreachable: "#d86b6b",
};

export function sessionPath(id: string): string {
  return `~/.apxm/sessions/${id}`;
}
