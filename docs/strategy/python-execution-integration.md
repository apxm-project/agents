# Python Frontend Execution Integration

**Status**: Design
**Date**: 2026-04-11
**Scope**: `crates/compiler/apxm-frontend/python/apxm/` → `crates/tools/apxm-server/`

## Problem Statement

The Python frontend can capture workflow graphs via `@compile()` but cannot
actually execute them. Every example in `examples/python/` ends with
`print(flow._graph.to_air())` — none of them run. The execution path in
`CompiledFlow.run()` is broken in two independent ways, and the architecture
forces unnecessary temp files to disk.

## Current Architecture

```
@compile()
def research(g: GraphRecorder, topic: str):
    g.ask("r", "Research {topic}")

# At decoration time:
#   1. Captures function body → ApxmGraph (in-memory Python dataclass)
#   2. Derives parameters from signature (topic: str → FlowParameter("topic", "str"))
#   3. Wraps in CompiledFlow

# At call time (await research("quantum")):
#   4. CompiledFlow.run("quantum")
#   5. Tries native import → FAILS (WorkflowBuilder doesn't exist)
#   6. Falls back to subprocess:
#      a. Writes graph to temp file on disk
#      b. Shells out: dekk apxm execute /tmp/xyz.air --json-output --input "quantum"
#      c. Parses stdout
#      d. Deletes temp file
```

### Why It Fails

**Bug 1 — CLI flags don't exist:**
The subprocess fallback (`_build_subprocess_cmd`) passes `--input` and
`--json-output`, but the Rust CLI has neither flag. The CLI uses positional
trailing args and a global `--json` flag.

```python
# execution.py line 327-332 (BROKEN)
cmd = [apxm_or_dekk, "apxm", "execute", tmp_path, "--json-output"]
cmd.extend(["--input", payload])

# CLI actually expects (cli.rs line 66-87):
# apxm execute <input> [args...] --json
```

**Bug 2 — Native module doesn't exist:**
`_ensure_native_compiled()` tries `importlib.import_module("apxm")` →
`WorkflowBuilder`, but this class was never implemented. No PyO3 or maturin
bindings exist in any Cargo.toml.

**Bug 3 — Arg serialization mismatch:**
`_serialize_args` flattens a single arg to string but JSON-encodes multiple
args. The Rust runtime expects individual positional strings, not a JSON blob.

## Wire Format Compatibility

The Python `ApxmGraph.to_dict()` output matches the Rust `AirModule` serde
format **exactly**. No field name mismatches:

| Component | Python JSON key | Rust AirModule field | Match |
|-----------|----------------|---------------------|-------|
| Node id | `"id"` | `AirNode.id: u64` | exact |
| Node name | `"name"` | `AirNode.name: String` | exact |
| Node op | `"op"` | `AirNode.op: AISOperationType` | exact |
| Node attrs | `"attributes"` | `AirNode.attributes: HashMap` | exact |
| Edge from | `"from"` | `AirEdge.from: u64` | exact |
| Edge to | `"to"` | `AirEdge.to: u64` | exact |
| Edge dep | `"dependency"` | `AirEdge.dependency: DependencyType` | exact |
| Param name | `"name"` | `AirParam.name: String` | exact |
| Param type | `"type_name"` | `AirParam.type_name: String` | exact |
| Graph name | `"name"` | `AirModule.name: String` | exact |
| Graph nodes | `"nodes"` | `AirModule.nodes: Vec<AirNode>` | exact |
| Graph edges | `"edges"` | `AirModule.edges: Vec<AirEdge>` | exact |
| Graph params | `"parameters"` | `AirModule.parameters: Vec<AirParam>` | exact |
| Graph meta | `"metadata"` | `AirModule.metadata: HashMap` | exact |

This means `graph.to_dict()` can be sent directly to the server's
`POST /v1/execute` endpoint with zero transformation.

## Server API (Already Exists)

The `apxm-server` at `localhost:18800` already has everything needed:

### POST /v1/execute

```python
# Request
{
    "graph": { ... },        # ApxmGraph.to_dict() — exact AirModule JSON
    "args": ["quantum"],     # Positional flow arguments
    "session_id": "s-123",   # Optional session for memory reuse
    "token_budget": 4096,    # Optional — injected into ASK nodes
    "output_schema": {...},  # Optional — JSON schema for output validation
}

# Response
{
    "results": {"0": "..."},  # token_id → output value
    "content": "...",         # First string result (convenience)
    "stats": {
        "executed_nodes": 3,
        "failed_nodes": 0,
        "duration_ms": 2451
    },
    "llm_usage": {
        "input_tokens": 150,
        "output_tokens": 420,
        "total_requests": 3
    }
}
```

### POST /v1/execute/stream

Same request body, returns SSE event stream with per-node progress:

```
event: data
data: {"type": "node_start", "node_id": 1, "name": "research", ...}

event: data
data: {"type": "node_complete", "node_id": 1, "output": "...", ...}

event: data
data: {"type": "execution_complete", "results": {...}, ...}
```

### Server Pipeline

```
ExecuteRequest.graph (JsonValue)
  → serde_json::from_value() → AirModule
  → apply_runtime_attributes() (inject token_budget, output_schema)
  → AirModule::to_air() → MLIR text
  → CompilerPipeline::compile() → Artifact
  → Runtime::execute_artifact_with_session() → RuntimeExecutionResult
  → to_execute_response() → ExecuteResponse
```

No files at any point. All in-memory.

## Source of Truth Audit

### What apxm-core owns (via codegen)

| Data | Rust Source | Codegen Target | Status |
|------|-----------|---------------|--------|
| Attribute keys | `apxm_ais::attrs::ALL_ATTR_NAMES` | `_generated/constants.py` | codegen'd |
| Operations | `apxm_ais::get_all_operations()` | `_generated/operations.py` | codegen'd |
| Models | `apxm_core::types::model_spec::BUILTIN_MODELS` | `_generated/models.py` | codegen'd |
| Providers | `apxm_core::types::provider_spec::BUILTIN_PROVIDERS` | `_generated/providers.py` | codegen'd |
| MLIR emission | `apxm_ais::EmissionSpec` | `_generated/emission.py` | codegen'd |
| LLM_OPS set | `category == "reasoning"` | `_generated/operations.py` | codegen'd |
| VOID_OPS set | `!produces_output` | `_generated/emission.py` | codegen'd |
| TEMPLATE_ATTRS | hardcoded in codegen.rs:268 | `_generated/emission.py` | codegen'd |

### What Python hardcodes (should come from core)

| Data | Python Location | Should Be |
|------|----------------|-----------|
| `_VALID_PARAM_TYPES = {"str","int","float","bool","json"}` | `ir.py:397` | codegen from core constant |
| `_PYTHON_TYPE_TO_APXM = {str: "str", int: "int", ...}` | `decorators.py:15-29` | target values from codegen |

### What Rust core is missing

`FlowParameter.type_name` is just a `String` — no canonical set of valid values
exists in Rust. The Python side hardcodes `{"str", "int", "float", "bool",
"json"}` in two places.

**Needed in `apxm-core/src/constants.rs`:**
```rust
pub mod parameters {
    pub const VALID_TYPES: &[&str] = &["str", "int", "float", "bool", "json"];
}
```

Then export via `registry.rs` → `render_constants_module()` → `_generated/constants.py`.

## Best Practices Research

### Anthropic SDK Pattern (reference architecture)

The Anthropic Python SDK uses a dual-class pattern that separates graph-building
(sync, no I/O) from execution (async, requires network):

- **`Anthropic`** (sync client) / **`AsyncAnthropic`** (async client)
- Both share a `BaseClient` with generics for the HTTP backend
- API key from env var (`ANTHROPIC_API_KEY`) with explicit override
- Context manager for connection cleanup
- Error hierarchy mapped to HTTP status codes
- Return types are dataclass/Pydantic models, not raw dicts
- Streaming via context manager yielding typed events

**Key insight for APXM:** Graph capture (`@compile`) is sync and pure — no I/O.
Execution is where the async/HTTP happens. These should be separate concerns.

### Error Hierarchy (Anthropic-inspired, adapted for APXM)

```
ApxmError (base)
  CompilationError          # graph validation or MLIR compile failure
  ExecutionError            # runtime failure (node crash, timeout)
  ServerError               # HTTP-level issues
    ConnectionError         # server not reachable
    TimeoutError            # request timeout
    ServerStatusError       # 4xx/5xx from server
```

**Current codebase convention:** Uses bare `ValueError` / `RuntimeError` with
descriptive messages. The new error types should follow the same style — no
complex exception classes, just clear hierarchical names with informative
messages.

### HTTP Client: httpx (optional dependency)

**Why httpx:**
- Same API for sync (`httpx.Client`) and async (`httpx.AsyncClient`)
- Built-in connection pooling, timeout control, streaming
- Used by the Anthropic SDK itself
- Supports SSE consumption via `httpx-sse`

**Dependency strategy:**
```toml
# pyproject.toml
[project]
dependencies = []  # core graph-building has zero deps

[project.optional-dependencies]
server = ["httpx>=0.25"]
stream = ["httpx>=0.25", "httpx-sse>=0.4"]
```

**Import guard pattern (matches existing codebase):**
```python
try:
    import httpx
except ImportError:
    httpx = None  # type: ignore[assignment,misc]
```

Error raised lazily, only when execution is attempted:
```python
if httpx is None:
    raise RuntimeError(
        "httpx is required for remote execution. "
        "Install it with: pip install apxm[server]"
    )
```

### Connection Management

**Long-lived client, not per-request:**
```python
# WRONG — defeats connection pooling
async def run(self):
    async with httpx.AsyncClient() as client:
        return await client.post(...)

# RIGHT — reuse client across calls
class CompiledFlow:
    _http: httpx.AsyncClient | None = None

    async def _get_client(self) -> httpx.AsyncClient:
        if self._http is None:
            self._http = httpx.AsyncClient(
                base_url=_server_url(),
                timeout=httpx.Timeout(connect=5.0, read=300.0, write=10.0, pool=5.0),
            )
        return self._http
```

**Timeout rationale:**
- `connect=5.0` — fail fast if server is down
- `read=300.0` — LLM workflows can take minutes
- `write=10.0` — graph JSON is small
- `pool=5.0` — don't wait long for connection slots

### Streaming (SSE)

```python
from httpx_sse import aconnect_sse

async def stream(self, *args):
    client = await self._get_client()
    async with aconnect_sse(
        client, "POST", "/v1/execute/stream",
        json=self._build_request(args),
        timeout=httpx.Timeout(connect=5.0, read=None, write=10.0, pool=5.0),
    ) as source:
        async for sse in source.aiter_sse():
            yield json.loads(sse.data)
```

`read=None` for SSE — the server may not send events for long periods during
LLM calls.

### Sync vs Async Callers

The `@compile()` decorator currently returns an `async def __call__`. This
forces all callers into async. Two approaches:

**Option A: Async-only (current, keep it)**
```python
# User must use asyncio
result = await research("topic")

# Or in __main__:
asyncio.run(research("topic"))
```

**Option B: Dual method (sync + async)**
```python
# Async
result = await research("topic")

# Sync (runs in thread to avoid nested loop issues)
result = research.run_sync("topic")
```

**Decision: Keep async-only for `__call__`, add `run_sync` convenience.**
The `run_sync` method uses a background thread with its own event loop,
avoiding the nested-loop pitfall in Jupyter/GUI contexts:

```python
def run_sync(self, *args, **kwargs):
    import threading
    result = None
    exc = None
    def _runner():
        nonlocal result, exc
        try:
            result = asyncio.run(self(*args, **kwargs))
        except Exception as e:
            exc = e
    t = threading.Thread(target=_runner)
    t.start()
    t.join()
    if exc is not None:
        raise exc
    return result
```

## Established Codebase Conventions (Must Follow)

Based on audit of existing Python frontend code:

| Convention | Pattern | Example |
|-----------|---------|---------|
| Future annotations | `from __future__ import annotations` on every file | universal |
| Union syntax | `str \| None` (modern, not `Optional[str]`) | all files |
| Dataclasses | `@dataclass(slots=True)`, mutable | `GraphNode`, `WorkflowCheckpoint` |
| Mutable defaults | `field(default_factory=dict)` or `None` + `__post_init__` | `WorkflowCheckpoint` |
| Errors | `ValueError` for validation, `RuntimeError` for execution | no custom hierarchy today |
| Import style | Mix relative/absolute; `from apxm._generated import constants as graph_keys` | all files |
| Optional imports | `try/except ImportError` with `None` fallback + `type: ignore` | `__init__.py` |
| Private methods | `_prefix` | `_add_node`, `_auto_name` |
| Method chaining | Return `self` from builder methods | `param()`, `register_tool()` |
| Docstrings | Brief description, optional `Returns:` section, indented examples | `proxy.py` |
| Tests | pytest, docstring on every test, imports inside test function | `test_decorators.py` |
| Constants | Always `graph_keys.CONSTANT_NAME`, never string literals | all files |
| `__all__` | Alphabetical, comprehensive | `__init__.py` |
| `__slots__` | On non-dataclass classes too | `CompiledFlow` |

## Proposed Architecture

### User Experience (No Files)

```python
from apxm import compile, GraphRecorder

@compile()
async def research(g: GraphRecorder, topic: str):
    r = g.ask("research", "Research {topic}")
    s = g.ask("summary", "Summarize: {research}")

# Async caller
result = await research("quantum computing")
print(result.content)

# Sync caller (convenience)
result = research.run_sync("quantum computing")
print(result.content)

# Optional: inspect graph without executing
print(research._graph.to_air())
research._compiled_flow.save("workflow.json")
```

### Execution Strategy (CompiledFlow.run)

```
1. HTTP (preferred, zero files, requires httpx)
   a. Check APXM_SERVER_URL env var or default http://localhost:18800
   b. POST /v1/execute with {"graph": graph.to_dict(), "args": [...]}
   c. Parse ExecuteResponse → return ExecutionResult
   d. On connection refused → fall through to #2

2. Subprocess via stdin (fallback, zero files, zero deps)
   a. Pipe graph JSON to stdin: echo <json> | dekk apxm execute /dev/stdin -- args
   b. Parse stdout JSON → return ExecutionResult
   c. On dekk/apxm not found → raise RuntimeError with install instructions

(Removed: temp file creation, --input flag, --json-output flag,
          native WorkflowBuilder import attempt)
```

### ExecutionResult (Return Type)

```python
@dataclass(slots=True)
class ExecutionStats:
    executed_nodes: int
    failed_nodes: int
    duration_ms: int

@dataclass(slots=True)
class LLMUsage:
    input_tokens: int
    output_tokens: int
    total_requests: int

@dataclass(slots=True)
class ExecutionResult:
    content: str | None
    results: dict[str, Any]
    stats: ExecutionStats
    llm_usage: LLMUsage

    @classmethod
    def from_response(cls, data: dict[str, Any]) -> ExecutionResult:
        """Parse server ExecuteResponse JSON."""
        return cls(
            content=data.get("content"),
            results=data.get("results", {}),
            stats=ExecutionStats(**data.get("stats", {})),
            llm_usage=LLMUsage(**data.get("llm_usage", {})),
        )
```

### Error Types

```python
class ApxmError(Exception):
    """Base exception for APXM operations."""

class CompilationError(ApxmError):
    """Graph failed validation or MLIR compilation."""

class ExecutionError(ApxmError):
    """Workflow execution failed at runtime."""

class ServerError(ApxmError):
    """Cannot communicate with apxm-server."""
```

Matches existing convention of descriptive messages:
```python
raise ServerError(
    "Cannot connect to apxm-server at http://localhost:18800.\n\n"
    "Start the server with: dekk apxm server\n"
    "Or set APXM_SERVER_URL to point to a running instance."
)
```

### HTTP Client Lifecycle

```python
# Module-level shared client (lazy, matches httpx best practice)
_client: httpx.AsyncClient | None = None

def _server_url() -> str:
    return os.environ.get("APXM_SERVER_URL", "http://localhost:18800")

async def _get_client() -> httpx.AsyncClient:
    global _client
    if _client is None:
        import httpx  # lazy import
        _client = httpx.AsyncClient(
            base_url=_server_url(),
            timeout=httpx.Timeout(connect=5.0, read=300.0, write=10.0, pool=5.0),
        )
    return _client
```

Module-level singleton avoids per-request overhead. The client is created lazily
on first execution, not at import time.

### Subprocess Stdin Fallback

```python
def _fallback_subprocess(self, *args: Any) -> ExecutionResult:
    apxm_bin = _find_apxm_binary()
    graph_json = json.dumps(self._graph.to_dict())

    if Path(apxm_bin).name == "dekk":
        cmd = [apxm_bin, "apxm", "execute", "/dev/stdin", "--json"]
    else:
        cmd = [apxm_bin, "execute", "/dev/stdin", "--json"]

    for arg in args:
        cmd.append(str(arg))

    result = subprocess.run(
        cmd, input=graph_json, capture_output=True, text=True, check=False
    )

    if result.returncode != 0:
        raise ExecutionError(
            f"apxm execute failed (exit {result.returncode}): {result.stderr.strip()}"
        )

    return ExecutionResult.from_response(json.loads(result.stdout))
```

Key changes from current code:
- Pipes JSON to stdin instead of writing temp file
- Uses `/dev/stdin` as the input path (Unix only; see Risks)
- Passes args as positional trailing args (not `--input`)
- Uses `--json` (not `--json-output`)
- Returns typed `ExecutionResult` instead of raw `Any`

## Implementation Plan

### Tier 1 — Source of Truth (apxm-core + codegen)

**File: `crates/core/apxm-core/src/constants.rs`**
- Add `pub mod parameters { pub const VALID_TYPES: &[&str] = &[...]; }`

**File: `crates/tools/apxm-cli/src/frontend/registry.rs`**
- Add `pub fn valid_param_types() -> &'static [&'static str]`

**File: `crates/tools/apxm-cli/src/frontend/codegen.rs`**
- In `render_constants_module()`: emit `VALID_PARAM_TYPES` frozenset
- In `render_constants_module()`: emit `PYTHON_TYPE_TO_APXM` dict

### Tier 2 — Python Execution (the real fix)

**New file: `crates/compiler/apxm-frontend/python/apxm/errors.py`**
- `ApxmError`, `CompilationError`, `ExecutionError`, `ServerError`

**File: `crates/compiler/apxm-frontend/python/apxm/execution.py`**
- Add `ExecutionResult`, `ExecutionStats`, `LLMUsage` dataclasses
- Rewrite `CompiledFlow.run()` → HTTP-first, subprocess-stdin fallback
- Add `CompiledFlow.run_sync()` — sync convenience method
- Add `CompiledFlow.stream()` → async generator via SSE
- Remove `_build_subprocess_cmd` temp file logic
- Remove `_ensure_native_compiled` (WorkflowBuilder doesn't exist)
- Remove `_fallback_retry_reason` (air vs json format retry hack)
- Remove `run_streaming` (replace with `stream()`)
- Add `session_id: str | None = None` parameter to `run()` and `stream()`
- Server URL: `os.environ.get("APXM_SERVER_URL", "http://localhost:18800")`
- Add `new_session() -> str` module-level helper

**File: `crates/compiler/apxm-frontend/python/apxm/decorators.py`**
- `_CompiledFunction.__call__` signature: `async def __call__(self, *args, session_id=None, **kwargs) -> ExecutionResult`
- Add `_CompiledFunction.run_sync()` sync convenience method
- Add `_CompiledFunction.stream()` async generator passthrough

**File: `crates/compiler/apxm-frontend/python/apxm/__init__.py`**
- Export `ExecutionResult`, `ExecutionStats`, `LLMUsage`
- Export `ApxmError`, `CompilationError`, `ExecutionError`, `ServerError`
- Export `new_session`
- Add to `__all__`

### Tier 3 — Eliminate Hardcoded Duplicates

**File: `crates/compiler/apxm-frontend/python/apxm/ir.py`**
- Replace hardcoded `_VALID_PARAM_TYPES` with import from `_generated.constants`

**File: `crates/compiler/apxm-frontend/python/apxm/decorators.py`**
- Replace `_PYTHON_TYPE_TO_APXM` target values with reference to generated set

### Tier 4 — pyproject.toml (optional dependencies)

**File: `crates/compiler/apxm-frontend/python/pyproject.toml`**
- Add `server = ["httpx>=0.25"]` to `[project.optional-dependencies]`
- Add `stream = ["httpx>=0.25", "httpx-sse>=0.4"]` to `[project.optional-dependencies]`

### Tier 5 — Tests

**File: `crates/compiler/apxm-frontend/python/tests/test_smoke.py`**
- Add `ExecutionResult`, `ExecutionStats`, `LLMUsage` to `required_exports` set
- Add `ApxmError`, `CompilationError`, `ExecutionError`, `ServerError` to `required_exports`
- Add `new_session` to `required_exports`

**New file: `crates/compiler/apxm-frontend/python/tests/test_execution.py`**
- Test `ExecutionResult.from_response()` with mock server response dict
- Test `new_session()` returns valid UUID string
- Test `CompiledFlow.run()` raises `ServerError` when server unreachable and
  no CLI binary found
- Test `_normalize_runtime_args` handles positional and keyword args
- Test `CompiledFlow.save()` and `CompiledFlow.load()` roundtrip

**Existing test files — verify no regressions:**
- `test_decorators.py` — decorator captures graph correctly (no changes needed,
  tests are pure graph construction)
- `test_graph_construction.py` — graph edges, nodes, params (no changes needed)
- `test_imports.py` — add new exports to import checks

### Tier 6 — Examples (40 files)

All 40 example files follow an identical pattern that needs updating:

**Current pattern (ALL files):**
```python
@compile()
def workflow_name(g: GraphRecorder):          # sync def
    # ... graph construction ...
    g.done(output)

if __name__ == "__main__":
    print(workflow_name._graph.to_air())       # only prints AIR, never runs
```

**New pattern:**
```python
@compile()
def workflow_name(g: GraphRecorder):          # stays sync def (graph capture is sync)
    # ... graph construction ...
    g.done(output)

if __name__ == "__main__":
    import asyncio
    # Show AIR (optional, for debugging)
    # print(workflow_name._graph.to_air())
    result = asyncio.run(workflow_name())
    print(result.content)
```

**Critical detail**: The `@compile()` decorated function itself stays `def` (NOT
`async def`). The function body is pure graph construction, not execution.
`_CompiledFunction.__call__` is the async part — it calls `CompiledFlow.run()`.

#### Files with parameters (2 files — special handling)

These files take arguments beyond `g: GraphRecorder` and need runtime args:

**`examples/python/real-world/code_review_council.py`:**
```python
@compile()
def code_review_council(g: GraphRecorder, code: str):
    # ...

if __name__ == "__main__":
    import asyncio
    sample_code = "def add(a, b): return a + b"
    result = asyncio.run(code_review_council(sample_code))
    print(result.content)
```

**`examples/python/real-world/ultrathink_coder.py`:**
```python
@compile()
def ultrathink_coder(g: GraphRecorder, task: str):
    # ...

if __name__ == "__main__":
    import asyncio
    result = asyncio.run(ultrathink_coder("implement a binary search"))
    print(result.content)
```

#### Files with g.param() (9 files — self-hosted workflows)

These files use `g.param()` inside the function body for graph-level parameters.
They need CLI-style positional args at runtime:

- `self-hosted/add_op.py` — params: `op_name`, `op_description`
- `self-hosted/remove_op.py` — params: `op_name`
- `self-hosted/explore.py` — params: `question`
- `self-hosted/plan_feature.py` — params: `feature`
- `self-hosted/refactor.py` — params: `target`, `goal`
- `self-hosted/autofix_workflow.py` — params: `scope`
- `self-hosted/audit.py` — params: `scope`

```python
@compile()
def explore_workflow(g: GraphRecorder):
    g.param("question", "str")
    # ...

if __name__ == "__main__":
    import asyncio
    result = asyncio.run(explore_workflow("How does the scheduler work?"))
    print(result.content)
```

#### Special case: `real-world/autofix_loop.py`

This file writes the graph to disk instead of printing AIR. It also writes
instructions for `dekk apxm execute`. Update to show both patterns:

```python
if __name__ == "__main__":
    import asyncio
    # Save graph for standalone execution
    autofix_loop._compiled_flow.save("autofix_loop.json")
    # Or execute directly
    result = asyncio.run(autofix_loop())
    print(result.content)
```

#### Benchmark files (10 files)

These are designed for `dekk apxm execute` benchmarking. Keep the `to_air()`
output as the primary behavior (benchmarks pipe AIR to the CLI). Add a
commented execution example:

```python
if __name__ == "__main__":
    print(chained_llm._graph.to_air())
    # To execute directly:
    # import asyncio
    # result = asyncio.run(chained_llm())
    # print(result.content)
```

### Complete File Inventory

| # | File | Change Type | Key Changes |
|---|------|------------|-------------|
| **Rust (Tier 1)** | | | |
| 1 | `apxm-core/src/constants.rs` | edit | Add `VALID_TYPES` |
| 2 | `apxm-cli/src/frontend/registry.rs` | edit | Export param types |
| 3 | `apxm-cli/src/frontend/codegen.rs` | edit | Emit `VALID_PARAM_TYPES`, `PYTHON_TYPE_TO_APXM` |
| **Python frontend (Tier 2-3)** | | | |
| 4 | `apxm/errors.py` | **new** | Error hierarchy |
| 5 | `apxm/execution.py` | **rewrite** | HTTP-first, subprocess-stdin fallback, result types |
| 6 | `apxm/decorators.py` | edit | Return `ExecutionResult`, add `run_sync`, `stream`, `session_id` |
| 7 | `apxm/__init__.py` | edit | Export new types and functions |
| 8 | `apxm/ir.py` | edit | Replace hardcoded `_VALID_PARAM_TYPES` |
| **Package config (Tier 4)** | | | |
| 9 | `pyproject.toml` | edit | Add `server` and `stream` optional deps |
| **Tests (Tier 5)** | | | |
| 10 | `tests/test_smoke.py` | edit | Verify new exports |
| 11 | `tests/test_imports.py` | edit | Import new types |
| 12 | `tests/test_execution.py` | **new** | `ExecutionResult`, `new_session`, error types |
| 13 | `tests/test_decorators.py` | no change | Pure graph construction (not affected) |
| 14 | `tests/test_graph_construction.py` | no change | Pure graph construction (not affected) |
| **Examples (Tier 6) — getting-started** | | | |
| 15 | `getting-started/hello.py` | edit | Add `asyncio.run()` execution |
| 16 | `getting-started/tool_use.py` | edit | Add `asyncio.run()` execution |
| **Examples — memory** | | | |
| 17 | `memory/rag_pipeline.py` | edit | Add `asyncio.run()` execution |
| **Examples — multi-agent** | | | |
| 18 | `multi-agent/negotiation.py` | edit | Add `asyncio.run()` execution |
| 19 | `multi-agent/parallel_agents.py` | edit | Add `asyncio.run()` execution |
| 20 | `multi-agent/spawn_and_communicate.py` | edit | Add `asyncio.run()` execution |
| 21 | `multi-agent/team_coordination.py` | edit | Add `asyncio.run()` execution |
| **Examples — parallelism** | | | |
| 22 | `parallelism/expert_council.py` | edit | Add `asyncio.run()` execution |
| 23 | `parallelism/worker_pool.py` | edit | Add `asyncio.run()` execution |
| 24 | `parallelism/fan_out_synthesize.py` | edit | Add `asyncio.run()` execution |
| **Examples — patterns** | | | |
| 25 | `patterns/cross_critique.py` | edit | Add `asyncio.run()` execution |
| 26 | `patterns/iterative_refine.py` | edit | Add `asyncio.run()` execution |
| 27 | `patterns/resilient_pipeline.py` | edit | Add `asyncio.run()` execution |
| **Examples — real-world** | | | |
| 28 | `real-world/code_review_council.py` | edit | Add `asyncio.run(fn(code))` with sample arg |
| 29 | `real-world/sdlc_pipeline.py` | edit | Add `asyncio.run()` execution |
| 30 | `real-world/codex_claude_fix.py` | edit | Add `asyncio.run()` execution |
| 31 | `real-world/ultrathink_coder.py` | edit | Add `asyncio.run(fn(task))` with sample arg |
| 32 | `real-world/autofix_loop.py` | edit | Add `asyncio.run()` + keep save pattern |
| **Examples — optimization** | | | |
| 33 | `optimization/fusion.py` | edit | Add `asyncio.run()` execution |
| 34 | `optimization/dead_context.py` | edit | Add `asyncio.run()` execution |
| 35 | `optimization/shared_prefix.py` | edit | Add `asyncio.run()` execution |
| 36 | `optimization/optimization_showcase.py` | edit | Add `asyncio.run()` execution |
| **Examples — multi-provider** | | | |
| 37 | `multi-provider/model_routing.py` | edit | Add `asyncio.run()` execution |
| **Examples — self-hosted** | | | |
| 38 | `self-hosted/add_op.py` | edit | Add `asyncio.run(fn(args))` with sample args |
| 39 | `self-hosted/remove_op.py` | edit | Add `asyncio.run(fn(arg))` with sample arg |
| 40 | `self-hosted/explore.py` | edit | Add `asyncio.run(fn(arg))` with sample arg |
| 41 | `self-hosted/plan_feature.py` | edit | Add `asyncio.run(fn(arg))` with sample arg |
| 42 | `self-hosted/refactor.py` | edit | Add `asyncio.run(fn(args))` with sample args |
| 43 | `self-hosted/autofix_workflow.py` | edit | Add `asyncio.run(fn(arg))` with sample arg |
| 44 | `self-hosted/audit.py` | edit | Add `asyncio.run(fn(arg))` with sample arg |
| **Examples — benchmarks (keep AIR output primary)** | | | |
| 45 | `_benchmarks/chained_llm.py` | edit | Add commented execution example |
| 46 | `_benchmarks/cse_stress.py` | edit | Add commented execution example |
| 47 | `_benchmarks/dead_context_stress.py` | edit | Add commented execution example |
| 48 | `_benchmarks/fusion_stress.py` | edit | Add commented execution example |
| 49 | `_benchmarks/memo_cache_stress.py` | edit | Add commented execution example |
| 50 | `_benchmarks/mixed_priority.py` | edit | Add commented execution example |
| 51 | `_benchmarks/multi_model.py` | edit | Add commented execution example |
| 52 | `_benchmarks/prefix_fanout_large.py` | edit | Add commented execution example |
| 53 | `_benchmarks/priority_scheduling.py` | edit | Add commented execution example |
| 54 | `_benchmarks/shared_prefix_fanout.py` | edit | Add commented execution example |

**Total: 54 files** (3 Rust + 9 Python frontend + 2 test + 40 examples)

### Files NOT changed (confirmed safe)

| File | Reason |
|------|--------|
| `apxm/config.py` | Pure dataclass definitions (AgentConfig, ToolsConfig) — no execution logic |
| `apxm/module.py` | FlowModule base class — pure graph capture, no execution |
| `apxm/sugar.py` | AgentHandle/Team — pure graph construction sugar |
| `apxm/proxy.py` | GraphRecorder — pure graph construction, no execution |
| `apxm/utils.py` | topological_sort, detect_cycle — pure algorithms |
| `apxm/normalize.py` | Value normalization — no execution dependency |
| `apxm/constants.py` | Re-exports from _generated — no changes needed |
| `apxm/providers.py` | Re-exports from _generated — no changes needed |
| `apxm/_generated/*.py` | Auto-generated by codegen (Tier 1 regenerates these) |
| `tests/test_decorators.py` | Pure graph capture tests (not affected) |
| `tests/test_graph_construction.py` | Pure graph construction tests (not affected) |
| `self-hosted/__init__.py` | Empty init file |

## Session and Execution ID Architecture

The runtime has **two distinct IDs** per execution:

### execution_id (always auto-generated)

Generated inside `ExecutionContext::new()` via `uuid::Uuid::now_v7()`. Every
single execution gets a unique `execution_id` automatically — this is never
user-controlled. It appears in:
- Execution event emissions
- Metrics and telemetry
- Session output directories (CLI: `{graph_name}-{timestamp}`)
- Scope registry entries

The Python frontend does **not** need to generate or manage this. The runtime
creates it internally.

### session_id (optional, for continuity)

An **optional** identifier that links multiple executions into a logical session.
When provided, it enables:
- **Memory continuity** — facts stored in one execution are queryable in the next
- **Session lane guard** — serializes concurrent executions within the same
  session (prevents race conditions on shared memory state)
- **Checkpoint persistence** — `SessionManager` saves checkpoint files keyed by
  session_id at `<checkpoint_dir>/<session_id>.json`

When `session_id` is `None` (the default), the execution is isolated — no
memory sharing, no lane serialization.

### How IDs flow through the system

```
Python frontend                    Server                        Runtime
─────────────────                  ──────                        ───────
session_id (optional)  ──HTTP──►  ExecuteRequest.session_id  ──►  build_context(session_id)
                                                                    ├── ctx.session_id = session_id
                                                                    └── ctx.execution_id = Uuid::now_v7()
                                                                        (auto-generated, always)
```

**CLI path** (`dekk apxm execute`): session_id is **not** passed from the CLI
to the runtime. The CLI generates its own `exec_id` for session output
(`{stem}-{timestamp}`) but this is an output directory name, not the runtime's
execution_id or session_id.

### Python frontend design

```python
@compile()
async def research(g: GraphRecorder, topic: str):
    g.ask("r", "Research {topic}")

# Isolated execution (default) — each call is independent
result = await research("quantum")

# Session-linked executions — memory carries across calls
result1 = await research("quantum", session_id="my-research")
result2 = await research("followup", session_id="my-research")
# result2's LLM nodes can access facts from result1

# Auto-generated session — Python generates a UUID once, reuses it
session = apxm.new_session()  # → "s-01970a4d-..."
result1 = await research("quantum", session_id=session)
result2 = await research("followup", session_id=session)
```

The `session_id` is passed through to the server's `ExecuteRequest.session_id`
field. The server passes it to `execute_artifact_with_session()`. The runtime
uses it for lane serialization and memory scoping. The `execution_id` is never
exposed to Python — it's an internal runtime concern.

### What _CompiledFunction.__call__ accepts

```python
async def __call__(self, *args: Any, session_id: str | None = None) -> ExecutionResult:
    runtime_args = self._normalize_runtime_args(*args)
    return await self._compiled_flow.run(*runtime_args, session_id=session_id)
```

`session_id` is a keyword-only argument that doesn't interfere with positional
flow parameters.

## Risks and Open Questions

1. **Server must be running for HTTP path.** The error message must be
   actionable: "Start the server with `dekk apxm server`". The subprocess
   fallback works without a server but requires the CLI binary.

2. **Subprocess stdin on non-Unix.** `/dev/stdin` is Unix-only. On Windows,
   fall back to temp file. This project targets Linux; document as known
   limitation.

3. **httpx as optional dependency.** Graph-building works with zero deps.
   Execution requires either httpx (for HTTP) or the CLI binary (for subprocess).
   If neither is available, raise `RuntimeError` with clear install instructions.

4. **Return type change.** `__call__` changes from `Any` to `ExecutionResult`.
   This is safe — the old path was broken, nobody depended on it.

5. **Session-less subprocess fallback.** The CLI `execute` command does not
   accept a `--session-id` flag. Subprocess executions are always isolated.
   If the user passes `session_id` and HTTP is unavailable, raise a clear
   error rather than silently dropping it.

6. **Module-level HTTP client.** Shared singleton avoids connection overhead but
   makes cleanup implicit. Add an `apxm.close()` function for explicit cleanup,
   and register an `atexit` handler for graceful shutdown.

7. **`@compile()` at import time.** Graph capture happens when the module is
   imported (decoration time). This is fine — it's pure computation, no I/O.
   No change needed.

8. **`g.done()` semantics.** All 40 examples use `g.done(node)` as the
   terminal call. This is a graph-construction operation that marks the exit
   node — it does NOT trigger execution. The execution happens at call time
   via `CompiledFlow.run()`. No semantic change to `g.done()` is needed.

9. **Benchmark pipeline dependency.** The 10 benchmark files
   (`examples/python/_benchmarks/`) are consumed by `scripts/benchmark_e2e.py`
   and `scripts/benchmark.py` which pipe AIR output to `dekk apxm execute`.
   Keep their `to_air()` output as the primary `__main__` behavior — do NOT
   replace it with `asyncio.run()`. Only add a commented execution example.

## File Index

| File | Role |
|------|------|
| `crates/core/apxm-core/src/constants.rs` | Add VALID_PARAM_TYPES constant |
| `crates/core/apxm-core/src/types/execution/dag.rs:15` | FlowParameter struct (unchanged) |
| `crates/tools/apxm-cli/src/frontend/registry.rs` | Export param types to codegen |
| `crates/tools/apxm-cli/src/frontend/codegen.rs` | Generate VALID_PARAM_TYPES in constants.py |
| `crates/tools/apxm-cli/src/commands/cli.rs:66-87` | CLI Execute command definition |
| `crates/tools/apxm-cli/src/commands/implementations.rs:1341` | execute_command (CLI entry) |
| `crates/tools/apxm-server/src/main.rs:482-502` | ExecuteRequest / ExecuteResponse structs |
| `crates/tools/apxm-server/src/main.rs:1001-1012` | execute handler |
| `crates/tools/apxm-server/src/main.rs:1015-1035` | execute_stream handler (SSE) |
| `crates/tools/apxm-server/src/main.rs:1159-1170` | prepare_request (JSON → AirModule) |
| `crates/tools/apxm-server/src/main.rs:1208-1230` | graph_to_artifact (AirModule → Artifact) |
| `crates/compiler/apxm-compiler/src/air_builder/mod.rs` | AirModule/AirNode/AirEdge/AirParam structs |
| `crates/runtime/apxm-runtime/src/runtime.rs:368-409` | execute_artifact_with_session_and_emitter |
| `crates/runtime/apxm-runtime/src/runtime.rs:496-516` | validate_args (param count check) |
| `crates/compiler/apxm-frontend/python/apxm/__init__.py` | Public API exports |
| `crates/compiler/apxm-frontend/python/apxm/decorators.py` | @compile, _CompiledFunction |
| `crates/compiler/apxm-frontend/python/apxm/execution.py` | CompiledFlow, ExecutionMode |
| `crates/compiler/apxm-frontend/python/apxm/ir.py` | ApxmGraph, to_air(), to_dict() |
| `crates/compiler/apxm-frontend/python/apxm/proxy.py` | GraphRecorder, NodeRef |
| `crates/compiler/apxm-frontend/python/apxm/sugar.py` | AgentHandle, Team |
| `crates/compiler/apxm-frontend/python/apxm/module.py` | FlowModule |
| `crates/compiler/apxm-frontend/python/apxm/_generated/constants.py` | Codegen'd constants |
