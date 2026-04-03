---
title: "Multi-Agent Execution"
description: "Runtime implementation for cross-agent communication and flow invocation"
---

# Multi-Agent Execution -- Implementation

For the parallelism model (unified scheduler graph, structural concurrency), see [Scheduling in PXMs](../../pxm/scheduling.md).
For COMM/FLOW instruction signatures and semantics, see [Communication Operations](../ais/communication.md).

This page covers runtime-specific implementation details.

## FlowRegistry

Artifact loading reconstructs runtime agents and registers them through
`FlowRegistry::register_agent()`, which stores:

- `agents: DashMap<String, Arc<Agent>>`
- `flows: DashMap<(String, String), Arc<ExecutionDag>>` (legacy lookup path for `FLOW`)

## Performance

| Workload               | Sequential | A-PXM  | Speedup |
|------------------------|-----------|--------|---------|
| 3-agent research       | 12.4s     | 1.2s   | 10.37x  |
| 2-agent code review    | 8.1s      | 2.3s   | 3.52x   |
| 5-agent data pipeline  | 31.0s     | 5.8s   | 5.34x   |

Gains come from critical-path compression: wall-clock time approaches the longest
dependency chain rather than the sum of all operations.

## Topology Example

```mermaid
flowchart LR
    subgraph Agent_A ["Agent A"]
        subgraph Flow_A ["Flow: main"]
            A1["Task: Fetch data"] --> A2["Task: Summarize"]
        end
    end

    subgraph Agent_B ["Agent B"]
        subgraph Flow_B ["Flow: main"]
            B1["Task: Fetch data"] --> B2["Task: Analyze"]
        end
    end

    subgraph Agent_C ["Agent C"]
        subgraph Flow_C ["Flow: main"]
            C1["Task: Merge results"] --> C2["Task: Report"]
        end
    end

    A2 -->|COMM| C1
    B2 -->|COMM| C1

    style Agent_A fill:#e8f4f8,stroke:#333
    style Agent_B fill:#f4e8f8,stroke:#333
    style Agent_C fill:#e8f8e8,stroke:#333
```

Agent A and Agent B execute fully in parallel. Agent C blocks only on the two
`COMM` edges, then proceeds immediately.

## Error Propagation

When an agent fails:

1. The runtime cancels all in-flight operations owned by that agent.
2. The error propagates as a standard `RuntimeError` through the scheduler.
   There is no special `AgentError` token type -- errors surface through the
   normal `OpState.last_error` / `first_error` mechanism in `SchedulerState`.
3. Receiving agents that depend on the failed agent's outputs will not fire
   (their input tokens remain unresolved).

## Process Management

Multi-agent execution is tracked via `ProcessTable` and `AgentProcess` (see
`crates/apxm-runtime/src/process_table.rs` and `process.rs`). Each spawned agent
gets a `ProcessId` (UUID v7), optional parent process link, and is indexed by
name for lookup. The `ProcessTable` enforces `max_processes` and
`max_spawn_depth` limits.
