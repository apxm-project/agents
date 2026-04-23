import * as EK from "./event-kinds";
import { truncate } from "./format";

export function eventSummary(payload: {
  kind: string;
  [key: string]: unknown;
}): string {
  const p = payload;
  const memoryScope = String(p.scope ?? p.tier ?? "");
  switch (p.kind) {
    case EK.TOKEN.name:
      return truncate(String(p.text ?? ""), 80);
    case EK.OPERATION_START.name:
      return `Node ${p.node_id} (${p.op_type}) started`;
    case "operation_complete":
      return `Node ${p.node_id} (${p.op_type}) done in ${p.duration_ms}ms`;
    case "operation_error":
      return `Node ${p.node_id} error: ${truncate(String(p.error ?? ""), 60)}`;
    case EK.SESSION_START.name:
      return `Session ${p.session_id} started`;
    case "session_complete":
      return `Session done in ${p.duration_ms}ms`;
    case "session_error":
      return `Session error: ${truncate(String(p.error ?? ""), 60)}`;
    case EK.SCHEDULER_DECISION.name:
      return `Scheduler: node ${p.node_id} delayed ${p.delay_ms}ms (${truncate(String(p.reason ?? ""), 40)})`;
    case EK.HEAD_OF_LINE_BLOCK.name:
      return `Head-of-line block: node ${p.blocked_node} waited ${p.wait_ms}ms behind node ${p.blocker_node}`;
    case EK.MEMORY_READ.name:
      return `Memory read: ${memoryScope}/${p.key}`;
    case EK.MEMORY_WRITE.name:
      return `Memory write: ${memoryScope}/${p.key}`;
    case "spawn_agent":
      return `Spawn agent: ${p.agent_type} (node ${p.node_id})`;
    case "agent_complete":
      return `Agent done: ${p.agent_type} (node ${p.node_id})`;
    case EK.CHECKPOINT_SAVED.name:
      return `Checkpoint: ${p.checkpoint_id}`;
    case EK.CHECKPOINT_RESTORED.name:
      return `Restored: ${p.checkpoint_id}`;
    case "capability_invoked":
      return `Capability: ${p.capability} (node ${p.node_id})`;
    case EK.RETRY.name:
      return `Retry #${p.attempt}: ${truncate(String(p.reason ?? ""), 40)}`;
    default:
      return p.kind;
  }
}
