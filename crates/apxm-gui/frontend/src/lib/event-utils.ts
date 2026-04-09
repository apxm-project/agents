import { truncate } from "./format";

export function eventSummary(payload: { kind: string; [key: string]: unknown }): string {
  const p = payload;
  switch (p.kind) {
    case "token":
      return truncate(String(p.text ?? ""), 80);
    case "operation_start":
      return `Node ${p.node_id} (${p.op_type}) started`;
    case "operation_complete":
      return `Node ${p.node_id} (${p.op_type}) done in ${p.duration_ms}ms`;
    case "operation_error":
      return `Node ${p.node_id} error: ${truncate(String(p.error ?? ""), 60)}`;
    case "session_start":
      return `Session ${p.session_id} started`;
    case "session_complete":
      return `Session done in ${p.duration_ms}ms`;
    case "session_error":
      return `Session error: ${truncate(String(p.error ?? ""), 60)}`;
    case "scheduler_decision":
      return `Scheduler: node ${p.node_id} \u2192 ${p.action}`;
    case "memory_read":
      return `Memory read: ${p.tier}/${p.key}`;
    case "memory_write":
      return `Memory write: ${p.tier}/${p.key}`;
    case "spawn_agent":
      return `Spawn agent: ${p.agent_type} (node ${p.node_id})`;
    case "agent_complete":
      return `Agent done: ${p.agent_type} (node ${p.node_id})`;
    case "checkpoint_created":
      return `Checkpoint: ${p.checkpoint_id}`;
    case "checkpoint_restored":
      return `Restored: ${p.checkpoint_id}`;
    case "capability_invoked":
      return `Capability: ${p.capability} (node ${p.node_id})`;
    case "retry":
      return `Retry #${p.attempt}: ${truncate(String(p.reason ?? ""), 40)}`;
    default:
      return p.kind;
  }
}
