# apxm-ais

Canonical AIS operation definitions shared by compiler and runtime.

## Overview

`apxm-ais` defines all 41 AIS operations, their metadata, 100+ attribute constants, validation rules, and MLIR pass descriptors. It is the single source of truth used to generate MLIR TableGen for the compiler and Rust metadata for the runtime dispatcher.

## Module Structure

| Module | Description |
|--------|-------------|
| `operations/` | `OperationSpec` definitions, categories, emission specs, TableGen generation |
| `attrs` | 100+ canonical attribute name constants (`agent_name`, `template_str`, `model`, etc.) |
| `passes/` | Pass descriptors and TableGen generation for MLIR optimization passes |
| `aam` | Agent Abstract Machine types (`AAM`, `Beliefs`, `Goals`, `Capabilities`) |
| `memory` | `MemoryTier` enum (STM, LTM, Episodic) |
| `types` | `Value` type used in operation parameters |
| `validation` | Operation field validation (`validate_operation`, `missing_required_fields`) |

## Operations (41 total)

| Category | Operations |
|----------|------------|
| Metadata | AGENT |
| Memory | QMEM, UMEM |
| LLM/Reasoning | ASK, THINK, REASON, PLAN, REFLECT, VERIFY |
| Tools | INV, EXC, PRINT |
| Control Flow | JUMP, BRANCH_ON_VALUE, LOOP_START, LOOP_END, RETURN, SWITCH, FLOW_CALL |
| Synchronization | MERGE, FENCE, WAIT_ALL, CHECKPOINT |
| Error Handling | TRY_CATCH, ERR |
| Communication | COMMUNICATE |
| Goal/State | UPDATE_GOAL, GUARD, CLAIM, PAUSE, RESUME |
| Coordination | DELEGATE, NEGOTIATE, SPAWN_AGENT, REGISTER_CAPABILITY, AUTONOMOUS |
| Identity | NOP, IDENTITY |
| Internal | CONST_STR, YIELD |

## Key Exports

- `AISOperationType` -- enum of all 41 operation types
- `OperationSpec` -- full metadata for one operation (fields, latency, emission spec)
- `get_operation_spec` / `get_all_operations` -- lookup functions
- `generate_tablegen` -- generates MLIR `.td` files from Rust definitions
- `generate_passes_tablegen` -- generates pass descriptors for the MLIR pipeline
- `AAM` / `Beliefs` / `Goals` / `Capabilities` -- Agent Abstract Machine types
- `Value` -- runtime value type
- `validate_operation` -- validates operation fields against spec

## Attribute Reference

Every attribute name used in operation specs, MLIR TableGen, runtime handlers, and the Python frontend is defined in `attrs.rs`. The `ALL_ATTR_NAMES` array lists every constant for consistency checking.

| Domain | Constants |
|--------|-----------|
| Agent/Identity | `agent_name`, `team_name`, `flow_name`, `profile`, `node_name`, `mode`, `cwd` |
| LLM/Model | `model`, `provider`, `temperature`, `system_prompt`, `token_budget`, `output_schema`, `max_schema_retries`, `backend`, `max_tool_iterations`, `budget` |
| Template/Prompt | `template_str`, `prompt`, `template` |
| Memory | `query`, `memory_tier`, `key`, `value`, `limit` |
| Capability/Tools | `capability`, `params_json`, `tools_enabled`, `tools`, `code`, `interpreter`, `capability_name`, `description`, `parameters_schema` |
| Communication | `message`, `recipient`, `target`, `protocol` |
| Goals/Reasoning | `goal`, `goal_id`, `priority`, `condition`, `evidence`, `claim` |
| Control Flow | `label`, `true_label`, `false_label`, `case_labels`, `try_label`, `catch_label`, `recovery_template` |
| Optimization Hints | `cached_system_prompt`, `memoizable`, `warmup_candidate`, `shared_prefix_est_tokens`, `downstream_nodes`, `reuse_group`, `est_template_tokens` |

## Dependencies

This crate has no internal APXM dependencies.
