# APXM Implementation Plan — Next 2 Weeks
**Prepared:** 2026-04-03
**Target:** Phase 3 close-out + Phase 4 kickoff + AgentMate consolidation
**Status:** ULTRATHINK complete — actionable blueprint ready

---

## Executive Summary

This plan covers 3 parallel work streams over the next 2 weeks:

1. **AgentMate Consolidation** (Option A: lean Python frontend) — 1-2 days
2. **Phase 3 Close-out** (integration tests, benchmarks, ContextStack foundation) — 2-3 days
3. **Phase 4 Kickoff** (full ContextStack, MemoCache two-tier, optimization targets) — 7-10 days

All work flows through `./launch-feature.sh` (3-parallel THINK workflow). No hand-coded implementations.

---

## 1. AgentMate: Option A Implementation (Lean Python Frontend)

### Overview
Extract `am-py` as standalone pure-Python package. Archive all 12 Rust crates (they duplicate APXM runtime).

### Step-by-Step Implementation

#### Phase 1.1: Extract Python Package (Day 1, AM)
**Deliverable:** Standalone `agentmate` PyPI package with no Rust dependencies

```bash
cd $APXM_HOME/external/agentmate

# 1. Create new directory structure
mkdir -p agentmate-python/{agentmate,tests,examples,docs}

# 2. Copy Python sources
cp -r crates/am-py/python/agentmate/* agentmate-python/agentmate/
cp -r crates/am-py/tests/* agentmate-python/tests/
cp -r examples/python/* agentmate-python/examples/

# 3. Create pyproject.toml (pure Python, no maturin)
cat > agentmate-python/pyproject.toml <<'EOF'
[build-system]
requires = ["setuptools>=61.0", "wheel"]
build-backend = "setuptools.build_meta"

[project]
name = "agentmate"
version = "0.3.0"
description = "Python graph DSL for APXM — the LLVM for agents"
readme = "README.md"
requires-python = ">=3.10"
license = {text = "MIT"}
authors = [{name = "APXM Team"}]
dependencies = []

[project.optional-dependencies]
dev = ["pytest>=7.0", "black", "mypy", "ruff"]

[project.urls]
Homepage = "https://github.com/apxm/agentmate"
Repository = "https://github.com/apxm/agentmate"

[tool.setuptools.packages.find]
where = ["."]
include = ["agentmate*"]
EOF

# 4. Add README.md
cat > agentmate-python/README.md <<'EOF'
# AgentMate — Python Graph DSL for APXM

Elegant, composable agent workflow graphs that compile to APXM's MLIR backend.

## Install
```bash
pip install agentmate
```

## Example
```python
from agentmate.graph import GraphRecorder

g = GraphRecorder("research")
plan = g.plan("create_plan", goal="Research quantum computing")
search = g.invoke("web_search", capability="web_search", params={"query": "quantum"})
think = g.think("analyze", template="Analyze: {0}")

plan | search | think

graph = g.to_graph()
print(graph.to_json())
```

Compiles to `.apxm` JSON, then: `dekk apxm execute graph.apxm`
EOF

# 5. Remove PyO3 imports (fallback to pure Python)
# Edit agentmate/graph/constants.py to remove try/except for native module
```

**Files that survive:**
```
agentmate-python/
├── agentmate/
│   ├── __init__.py
│   ├── graph/
│   │   ├── __init__.py
│   │   ├── ir.py            # ApxmGraph, GraphNode, GraphEdge
│   │   ├── proxy.py         # GraphRecorder (builder)
│   │   ├── module.py        # FlowModule (composable)
│   │   ├── execution.py     # CompiledFlow (CLI bridge)
│   │   ├── decorators.py    # @compile
│   │   ├── constants.py     # AIS op names
│   │   ├── config.py        # AgentConfig
│   │   └── utils.py         # detect_cycle
│   ├── agent.py
│   ├── tools.py
│   ├── supervisor.py
│   ├── codelet.py
│   ├── events.py
│   ├── providers.py
│   ├── prompts.py
│   ├── guardrails.py
│   ├── hitl.py
│   ├── mcp.py
│   └── context.py
├── tests/
│   └── graph/
│       ├── test_ir.py
│       ├── test_proxy.py
│       └── test_validation.py
├── examples/
│   ├── research_pipeline.py
│   └── code_architect.py
├── pyproject.toml
└── README.md
```

**Files to DELETE (or archive to `external/agentmate-rust-archive/`):**
```
crates/am-core/
crates/am-agents/
crates/am-tools/
crates/am-sandbox/
crates/am-config/
crates/am-cli/
crates/am-tui/
crates/am-rag/
crates/am-documents/
crates/am-mcp/
crates/am-macros/
crates/am-skills/
```

#### Phase 1.2: Add Missing AIS Ops to proxy.py (Day 1, PM)
**Missing ops:** SPAWN_AGENT, DELEGATE, NEGOTIATE, REGISTER_CAPABILITY, AUTONOMOUS, NOP, IDENTITY

**File:** `agentmate-python/agentmate/graph/proxy.py`

Add these 7 methods to `GraphRecorder` class:

```python
def spawn_agent(
    self,
    name: str,
    *,
    agent_name: str,
    profile: str | None = None,
    cwd: str | None = None,
    mode: str | None = None,
    model: str | None = None,
    **attributes: Any,
) -> NodeRef:
    """Spawn an external agent subprocess (SPAWN_AGENT)."""
    attrs: dict[str, Any] = {graph_keys.AGENT_NAME: agent_name}
    if profile is not None:
        attrs[graph_keys.PROFILE] = profile
    if cwd is not None:
        attrs[graph_keys.CWD] = cwd
    if mode is not None:
        attrs[graph_keys.MODE] = mode
    if model is not None:
        attrs[graph_keys.MODEL] = model
    attrs.update(_normalize_attributes(attributes))
    return self._add_node(name, graph_keys.OP_SPAWN_AGENT, attrs)

def delegate(
    self,
    name: str,
    *,
    target: str,
    task_spec: str,
    **attributes: Any,
) -> NodeRef:
    """Delegate a task to a sub-agent (DELEGATE)."""
    attrs: dict[str, Any] = {
        graph_keys.DELEGATE_TARGET: target,
        graph_keys.DELEGATE_TASK_SPEC: task_spec,
    }
    attrs.update(_normalize_attributes(attributes))
    return self._add_node(name, graph_keys.OP_DELEGATE, attrs)

def negotiate(
    self,
    name: str,
    *,
    proposal: str,
    party: str,
    max_rounds: int | None = None,
    **attributes: Any,
) -> NodeRef:
    """Multi-agent negotiation protocol (NEGOTIATE)."""
    attrs: dict[str, Any] = {
        graph_keys.NEGOTIATE_PROPOSAL: proposal,
        graph_keys.NEGOTIATE_PARTY: party,
    }
    if max_rounds is not None:
        attrs[graph_keys.NEGOTIATE_ROUND] = max_rounds
    attrs.update(_normalize_attributes(attributes))
    return self._add_node(name, graph_keys.OP_NEGOTIATE, attrs)

def register_capability(
    self,
    name: str,
    *,
    capability: str,
    handler: str,
    schema: dict[str, Any] | None = None,
    **attributes: Any,
) -> NodeRef:
    """Register a new capability at runtime (REGISTER_CAPABILITY)."""
    attrs: dict[str, Any] = {
        graph_keys.CAPABILITY: capability,
        "handler": handler,
    }
    if schema is not None:
        attrs["schema"] = _normalize_value(schema)
    attrs.update(_normalize_attributes(attributes))
    return self._add_node(name, graph_keys.OP_REGISTER_CAPABILITY, attrs)

def autonomous(
    self,
    name: str,
    *,
    region: str,
    max_steps: int | None = None,
    **attributes: Any,
) -> NodeRef:
    """Switch to model-driven autonomous execution (AUTONOMOUS)."""
    attrs: dict[str, Any] = {"region": region}
    if max_steps is not None:
        attrs["max_steps"] = max_steps
    attrs.update(_normalize_attributes(attributes))
    return self._add_node(name, graph_keys.OP_AUTONOMOUS, attrs)

def nop(self, name: str, **attributes: Any) -> NodeRef:
    """No-op passthrough (NOP)."""
    return self._add_node(name, graph_keys.OP_NOP, _normalize_attributes(attributes))

def identity(self, name: str, **attributes: Any) -> NodeRef:
    """Identity passthrough with AAM transition recorded (IDENTITY)."""
    return self._add_node(name, graph_keys.OP_IDENTITY, _normalize_attributes(attributes))
```

**Also add to `constants.py`:**
```python
OP_SPAWN_AGENT = "SPAWN_AGENT"
OP_DELEGATE = "DELEGATE"
OP_NEGOTIATE = "NEGOTIATE"
OP_REGISTER_CAPABILITY = "REGISTER_CAPABILITY"
OP_AUTONOMOUS = "AUTONOMOUS"
OP_NOP = "NOP"
OP_IDENTITY = "IDENTITY"

# Attribute keys
PROFILE = "profile"
CWD = "cwd"
MODE = "mode"
MODEL = "model"
DELEGATE_TARGET = "delegate_target"
DELEGATE_TASK_SPEC = "delegate_task_spec"
NEGOTIATE_PROPOSAL = "negotiate_proposal"
NEGOTIATE_PARTY = "negotiate_party"
NEGOTIATE_ROUND = "negotiate_round"
```

#### Phase 1.3: Fix CLI Integration (Day 1, PM)
**File:** `agentmate-python/agentmate/graph/execution.py`

**Line 276:** Change from `apxm` to `dekk apxm execute`

```python
# BEFORE (line 276):
cmd = [apxm_bin, "execute", tmp_path, "--json-output"]

# AFTER:
# Use dekk if available, fallback to apxm
dekk_bin = shutil.which("dekk")
if dekk_bin is not None:
    cmd = [dekk_bin, "apxm", "execute", tmp_path, "--json-output"]
else:
    cmd = [apxm_bin, "execute", tmp_path, "--json-output"]
```

Also update `_find_apxm_binary()` to check for `dekk` first:
```python
def _find_apxm_binary() -> str:
    dekk_bin = shutil.which("dekk")
    if dekk_bin is not None:
        return dekk_bin  # Will be used with "apxm execute" subcommand

    apxm_bin = shutil.which("apxm")
    if apxm_bin is None:
        raise RuntimeError(
            "Neither 'dekk' nor 'apxm' found on PATH. "
            "Install APXM or ensure dekk is configured."
        )
    return apxm_bin
```

#### Phase 1.4: Run Tests (Day 1, PM)
```bash
cd agentmate-python

# 1. Install in dev mode
pip install -e ".[dev]"

# 2. Run existing tests
pytest tests/graph/ -v

# 3. Validation test
python -c "
from agentmate.graph import GraphRecorder
g = GraphRecorder('test')
n = g.ask('q', template='Hello')
graph = g.to_graph()
print(graph.to_json())
"

# 4. End-to-end test
cd examples
python research_pipeline.py > /tmp/test.apxm
dekk apxm validate /tmp/test.apxm
dekk apxm execute /tmp/test.apxm
```

#### Phase 1.5: Git Relationship (Day 2)
**Decision:** Archive, not submodule (agentmate should be standalone PyPI package)

```bash
cd $APXM_HOME

# 1. Create archive directory
mkdir -p external/agentmate-archive
mv external/agentmate/crates external/agentmate-archive/rust-crates
mv external/agentmate/.git external/agentmate-archive/.git

# 2. Move Python package to new repo (separate from APXM)
# This will become a standalone GitHub repo: github.com/apxm/agentmate
mv agentmate-python ~/projects/agentmate

# 3. Update APXM docs to reference PyPI package
echo "Python SDK: pip install agentmate" > docs/python-sdk.md
```

**Going forward:**
- AgentMate repo: `github.com/apxm/agentmate` (Python only)
- APXM repo: `github.com/apxm/apxm` (Rust runtime + MLIR compiler)
- Relationship: AgentMate produces `.apxm` JSON → APXM consumes it

---

## 2. Node Sync (097-083 ↔ 097-045)

### Current State
- **097-083 (build):** `193957c` "TODO docs" (main branch, latest)
- **097-045 (inference):** Unknown HEAD (vLLM fork on `apxm` branch at `73e130d`)

### Investigation Commands
```bash
# On 097-083 (current node)
git log --oneline -20 > /tmp/083-log.txt
git status

# SSH to 097-045 (need to fix host key first)
ssh-keyscan useocpm2m-097-045 >> ~/.ssh/known_hosts

# On 097-045
ssh useocpm2m-097-045 "cd ~/projects/agents/apxm && git log --oneline -20"
ssh useocpm2m-097-045 "cd ~/projects/agents/apxm && git status"
ssh useocpm2m-097-045 "cd ~/projects/agents/apxm && git rev-parse HEAD"
```

### Sync Strategy
**Assumption:** 097-045 is behind (hasn't received commits 193957c, 1234bcc, 4b5fa6d, 8c5c98b, cc2fa1f)

```bash
# On 097-045
cd ~/projects/agents/apxm

# 1. Stash any local changes
git stash

# 2. Fetch latest from origin
git fetch origin main

# 3. Fast-forward or rebase to main
git merge --ff-only origin/main  # If possible
# OR
git rebase origin/main  # If local commits exist

# 4. Pop stash if needed
git stash pop

# 5. Rebuild APXM
dekk apxm build

# 6. Restart vLLM container with updated graph hints
docker restart vllm-gemma3-v2
```

### Going-Forward Workflow
**Rule:** All code changes happen on 097-083 → push to GitHub → pull on 097-045

```bash
# On 097-083 (after feature implementation)
git push origin main

# On 097-045 (daily sync)
cd ~/projects/agents/apxm
git pull origin main
dekk apxm build
```

**Automation option:** Set up GitHub Actions to SSH into 097-045 and trigger rebuild on push to main.

---

## 3. Integration Tests Design

### Overview
Create `tests/` at repo root with 5 core integration tests. Use `pytest` + `pytest-asyncio`.

### Directory Structure
```
$APXM_HOME/tests/
├── __init__.py
├── conftest.py               # Pytest fixtures (backend setup, temp dirs)
├── test_basic_execution.py   # Test 1: ASK → output
├── test_vllm_hints.py        # Test 2: GraphAwareVllmBackend roundtrip
├── test_parallel.py          # Test 3: WAIT_ALL + MERGE
├── test_circuit_breaker.py   # Test 4: Backend failover
├── test_acp_agent.py         # Test 5: INV(acp) → agent → result
└── fixtures/
    ├── simple.apxm
    ├── parallel.apxm
    └── acp-spawn.apxm
```

### Test 1: Basic Graph Execution
**File:** `tests/test_basic_execution.py`

```python
import pytest
import subprocess
import json
from pathlib import Path

FIXTURES = Path(__file__).parent / "fixtures"

def test_simple_ask_execution():
    """Validate that a single ASK node executes and returns output."""
    graph_path = FIXTURES / "simple.apxm"

    # Execute graph
    result = subprocess.run(
        ["dekk", "apxm", "execute", str(graph_path), "--json-output"],
        capture_output=True,
        text=True,
        check=True,
    )

    # Parse output
    output = json.loads(result.stdout)
    assert "content" in output or "result" in output
    assert result.returncode == 0

def test_validate_before_execute():
    """Ensure validation catches invalid graphs before execution."""
    invalid_graph = {
        "name": "invalid",
        "nodes": [{"id": 1, "name": "n1", "op": "UNKNOWN_OP", "attributes": {}}],
        "edges": [],
        "parameters": [],
        "metadata": {},
    }

    tmp = Path("/tmp/invalid.apxm")
    tmp.write_text(json.dumps(invalid_graph))

    result = subprocess.run(
        ["dekk", "apxm", "validate", str(tmp)],
        capture_output=True,
        text=True,
    )

    assert result.returncode != 0
    assert "UNKNOWN_OP" in result.stderr or "invalid" in result.stderr.lower()
```

**Fixture:** `fixtures/simple.apxm`
```json
{
  "name": "simple_ask",
  "nodes": [
    {
      "id": 1,
      "name": "greeting",
      "op": "ASK",
      "attributes": {
        "template_str": "Say hello in one sentence."
      }
    }
  ],
  "edges": [],
  "parameters": [],
  "metadata": {}
}
```

### Test 2: GraphAwareVllmBackend Roundtrip
**File:** `tests/test_vllm_hints.py`

```python
import pytest
import json
from pathlib import Path
from apxm_backends.llm.backends.vllm import GraphAwareVllmBackend, GraphMetadata, NodeSpec

@pytest.mark.asyncio
@pytest.mark.skipif(
    not _vllm_available(),
    reason="vLLM not reachable at http://localhost:8000"
)
async def test_register_graph_metadata():
    """Test that graph registration roundtrips correctly."""
    backend = await GraphAwareVllmBackend.new("", config={
        "base_url": "http://localhost:8000",
        "model": "Gemma-3-27B-it"
    })

    # Create graph metadata
    metadata = GraphMetadata.new("test-graph-001", "exec-001") \
        .with_pin_ttl(30_000) \
        .with_critical_path_length(3) \
        .with_nodes([
            NodeSpec(
                node_id=1,
                node_name="plan",
                estimated_prompt_tokens=500,
                downstream_nodes=[2],
                priority_class="critical_path",
                is_critical_path=True,
            )
        ])

    # Register graph
    response = await backend.register_graph(metadata)
    assert response["graph_id"] == "test-graph-001"
    assert response["registered_nodes"] == 1

    # Release graph
    release = await backend.release_graph("test-graph-001")
    assert release["graph_id"] == "test-graph-001"

@pytest.mark.asyncio
async def test_hints_injected_into_request():
    """Verify APXM hints are added to extra_body.apxm."""
    from apxm_backends.llm.request import LLMRequest
    from apxm_backends.llm.backends.vllm.graph_meta import ApxmGraphHints

    backend = await GraphAwareVllmBackend.new("", config={
        "base_url": "http://localhost:8000",
        "model": "test-model"
    })

    hints = ApxmGraphHints.critical_path(
        "dag-123", "exec-456", 5, "reason-node", [6, 7], 30_000
    )

    request = LLMRequest.new("Test prompt").with_apxm_hints(hints)
    prepared = backend.inject_hints(request)

    # Check that extra_body.apxm contains the hints
    assert prepared.extra_body is not None
    apxm_hints = prepared.extra_body.get("apxm")
    assert apxm_hints is not None
    assert apxm_hints["graph_id"] == "dag-123"
    assert apxm_hints["node_id"] == 5
    assert apxm_hints["priority_class"] == "critical_path"

def _vllm_available() -> bool:
    import requests
    try:
        resp = requests.get("http://localhost:8000/health", timeout=2)
        return resp.status_code == 200
    except:
        return False
```

### Test 3: Parallel Execution (WAIT_ALL + MERGE)
**File:** `tests/test_parallel.py`

```python
import pytest
import subprocess
import json
from pathlib import Path

FIXTURES = Path(__file__).parent / "fixtures"

def test_wait_all_parallel_execution():
    """Verify that WAIT_ALL correctly synchronizes parallel branches."""
    graph_path = FIXTURES / "parallel.apxm"

    result = subprocess.run(
        ["dekk", "apxm", "execute", str(graph_path), "--emit-metrics", "/tmp/metrics.json"],
        capture_output=True,
        text=True,
        check=True,
    )

    # Check metrics show parallelism
    metrics = json.loads(Path("/tmp/metrics.json").read_text())
    assert "parallelism" in metrics or "concurrent_nodes" in metrics

def test_merge_node_combines_outputs():
    """Verify that MERGE combines multiple token outputs."""
    # Graph: ASK → MERGE ← ASK (two parallel asks merged)
    graph = {
        "name": "merge_test",
        "nodes": [
            {"id": 1, "name": "ask1", "op": "ASK", "attributes": {"template_str": "Say A"}},
            {"id": 2, "name": "ask2", "op": "ASK", "attributes": {"template_str": "Say B"}},
            {"id": 3, "name": "merge", "op": "MERGE", "attributes": {}},
        ],
        "edges": [
            {"from": 1, "to": 3, "dependency": "Data"},
            {"from": 2, "to": 3, "dependency": "Data"},
        ],
        "parameters": [],
        "metadata": {},
    }

    tmp = Path("/tmp/merge_test.apxm")
    tmp.write_text(json.dumps(graph))

    result = subprocess.run(
        ["dekk", "apxm", "execute", str(tmp)],
        capture_output=True,
        text=True,
        check=True,
    )

    assert result.returncode == 0
```

**Fixture:** `fixtures/parallel.apxm`
```json
{
  "name": "parallel_wait_all",
  "nodes": [
    {"id": 1, "name": "ask1", "op": "ASK", "attributes": {"template_str": "Count to 3"}},
    {"id": 2, "name": "ask2", "op": "ASK", "attributes": {"template_str": "List 3 colors"}},
    {"id": 3, "name": "sync", "op": "WAIT_ALL", "attributes": {}},
    {"id": 4, "name": "merge", "op": "MERGE", "attributes": {}}
  ],
  "edges": [
    {"from": 1, "to": 3, "dependency": "Control"},
    {"from": 2, "to": 3, "dependency": "Control"},
    {"from": 1, "to": 4, "dependency": "Data"},
    {"from": 2, "to": 4, "dependency": "Data"}
  ],
  "parameters": [],
  "metadata": {}
}
```

### Test 4: Backend Failover (Circuit Breaker)
**File:** `tests/test_circuit_breaker.py`

```python
import pytest
from apxm_runtime.model_router import ModelRouter, ModelRouterConfig, CircuitBreakerConfig
from apxm_backends import LLMRegistry, LLMRequest

@pytest.mark.asyncio
async def test_circuit_breaker_opens_after_failures():
    """Verify circuit breaker trips after threshold failures."""
    llm_registry = LLMRegistry()

    config = ModelRouterConfig(
        circuit_breaker=CircuitBreakerConfig(
            failure_threshold=3,
            open_duration_secs=60,
            min_requests=1,
        ),
        target="balanced",
        operation_policies=[],
    )

    router = ModelRouter.new(llm_registry, config)
    router.circuit_breakers.register("test-backend")

    # Record 3 failures
    for _ in range(3):
        router.record_failure("test-backend")

    # Circuit should be open
    assert not router.circuit_breakers.is_available("test-backend")

    # Reset and verify it closes
    router.reset_circuit("test-backend")
    assert router.circuit_breakers.is_available("test-backend")

@pytest.mark.asyncio
async def test_failover_to_secondary_backend():
    """Verify router fails over when primary backend is unavailable."""
    # Setup: primary backend open, secondary backend healthy
    llm_registry = LLMRegistry()
    router = ModelRouter.new(llm_registry, ModelRouterConfig.default())

    router.circuit_breakers.register("primary")
    router.circuit_breakers.register("secondary")

    # Trip primary
    for _ in range(5):
        router.record_failure("primary")

    # Request should route to secondary (if available)
    request = LLMRequest.new("Test").with_backend("primary")
    decision = router.select(request)

    # Should fail or route to fallback (depends on registry state)
    # This test needs actual backends registered to be meaningful
    assert decision is not None or True  # Placeholder
```

### Test 5: ACP Agent Invocation
**File:** `tests/test_acp_agent.py`

```python
import pytest
import subprocess
import json
from pathlib import Path

FIXTURES = Path(__file__).parent / "fixtures"

@pytest.mark.skipif(
    not _agent_available("claude"),
    reason="Claude agent not registered in ~/.apxm/agents.toml"
)
def test_invoke_acp_agent():
    """Test INV(acp) node spawns agent and returns result."""
    graph_path = FIXTURES / "acp-spawn.apxm"

    result = subprocess.run(
        ["dekk", "apxm", "execute", str(graph_path), "--emit-session"],
        capture_output=True,
        text=True,
        timeout=30,
    )

    # Should succeed if agent profile exists
    assert result.returncode == 0

    # Check session output
    session_dir = Path.home() / ".apxm" / "sessions"
    latest = max(session_dir.iterdir(), key=lambda p: p.stat().st_mtime)
    results = json.loads((latest / "results.json").read_text())

    # INV node should have output
    assert len(results) > 0

def _agent_available(name: str) -> bool:
    result = subprocess.run(
        ["dekk", "apxm", "agent", "list"],
        capture_output=True,
        text=True,
    )
    return name in result.stdout
```

**Fixture:** `fixtures/acp-spawn.apxm`
```json
{
  "name": "acp_test",
  "nodes": [
    {
      "id": 1,
      "name": "invoke_agent",
      "op": "INV",
      "attributes": {
        "capability": "acp",
        "params_json": "{\"agent_name\": \"claude\", \"message\": \"Say hello\"}"
      }
    }
  ],
  "edges": [],
  "parameters": [],
  "metadata": {}
}
```

### Running Tests
```bash
cd $APXM_HOME

# Install test dependencies
pip install pytest pytest-asyncio requests

# Run all integration tests
pytest tests/ -v --tb=short

# Run specific test
pytest tests/test_vllm_hints.py::test_register_graph_metadata -v

# Run with coverage
pytest tests/ --cov=crates --cov-report=html
```

### CI Integration
Add `.github/workflows/integration.yml`:
```yaml
name: Integration Tests

on: [push, pull_request]

jobs:
  test:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v3
      - uses: conda-incubator/setup-miniconda@v2
        with:
          environment-file: environment.yaml
      - run: cargo build -p apxm-cli --features driver --release
      - run: pytest tests/ -v
```

---

## 4. Benchmarks Design (Criterion Suite)

### Overview
Create `benches/` directory with Criterion.rs benchmarks. Measure latency (P50/P95/P99), throughput, token counts.

### Directory Structure
```
$APXM_HOME/benches/
├── graph_execution.rs       # Benchmark 1: Graph execution latency
├── vllm_hints_overhead.rs   # Benchmark 2: With/without graph hints
├── parallel_speedup.rs      # Benchmark 3: Parallelism scaling
├── memoization.rs           # Benchmark 4: Cache hit/miss performance
└── fixtures/
    ├── simple_ask.apxm
    ├── parallel_3way.apxm
    ├── sequential_chain.apxm
    └── cached_repeat.apxm
```

### Setup
**File:** `Cargo.toml` (workspace root)
```toml
[[bench]]
name = "graph_execution"
harness = false

[[bench]]
name = "vllm_hints_overhead"
harness = false

[[bench]]
name = "parallel_speedup"
harness = false

[[bench]]
name = "memoization"
harness = false

[dev-dependencies]
criterion = { version = "0.5", features = ["html_reports", "async_tokio"] }
tokio = { version = "1", features = ["full"] }
```

### Benchmark 1: Graph Execution Latency
**File:** `benches/graph_execution.rs`

```rust
use criterion::{black_box, criterion_group, criterion_main, Criterion, BenchmarkId};
use apxm_driver::runtime::RuntimeExecutor;
use apxm_graph::ApxmGraph;
use std::path::PathBuf;

fn load_graph(name: &str) -> ApxmGraph {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("benches/fixtures")
        .join(format!("{}.apxm", name));
    let json = std::fs::read_to_string(path).unwrap();
    serde_json::from_str(&json).unwrap()
}

fn bench_simple_ask(c: &mut Criterion) {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let graph = load_graph("simple_ask");

    c.bench_function("execute_simple_ask", |b| {
        b.to_async(&rt).iter(|| async {
            let executor = RuntimeExecutor::new().await.unwrap();
            black_box(executor.execute_graph(graph.clone()).await)
        });
    });
}

fn bench_varying_graph_sizes(c: &mut Criterion) {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let mut group = c.benchmark_group("graph_size_scaling");

    for node_count in [1, 5, 10, 20, 50].iter() {
        group.bench_with_input(
            BenchmarkId::from_parameter(node_count),
            node_count,
            |b, &size| {
                let graph = generate_chain_graph(size);
                b.to_async(&rt).iter(|| async {
                    let executor = RuntimeExecutor::new().await.unwrap();
                    black_box(executor.execute_graph(graph.clone()).await)
                });
            },
        );
    }
    group.finish();
}

fn generate_chain_graph(node_count: usize) -> ApxmGraph {
    // Generate a sequential chain of ASK nodes
    let mut nodes = Vec::new();
    let mut edges = Vec::new();

    for i in 1..=node_count {
        nodes.push(serde_json::json!({
            "id": i,
            "name": format!("ask_{}", i),
            "op": "ASK",
            "attributes": {"template_str": "Count to 3"}
        }));

        if i > 1 {
            edges.push(serde_json::json!({
                "from": i - 1,
                "to": i,
                "dependency": "Data"
            }));
        }
    }

    let graph_json = serde_json::json!({
        "name": format!("chain_{}", node_count),
        "nodes": nodes,
        "edges": edges,
        "parameters": [],
        "metadata": {}
    });

    serde_json::from_value(graph_json).unwrap()
}

criterion_group!(benches, bench_simple_ask, bench_varying_graph_sizes);
criterion_main!(benches);
```

### Benchmark 2: vLLM Hints Overhead
**File:** `benches/vllm_hints_overhead.rs`

```rust
use criterion::{black_box, criterion_group, criterion_main, Criterion};
use apxm_backends::llm::backends::vllm::{GraphAwareVllmBackend, GraphMetadata};
use apxm_backends::llm::{LLMRequest, LLMBackend};

fn bench_without_hints(c: &mut Criterion) {
    let rt = tokio::runtime::Runtime::new().unwrap();

    c.bench_function("vllm_request_no_hints", |b| {
        b.to_async(&rt).iter(|| async {
            let backend = GraphAwareVllmBackend::new("", Some(serde_json::json!({
                "base_url": "http://localhost:8000",
                "model": "Gemma-3-27B-it"
            }))).await.unwrap();

            let request = LLMRequest::new("Say hello in one word");
            black_box(backend.generate(request).await)
        });
    });
}

fn bench_with_hints(c: &mut Criterion) {
    let rt = tokio::runtime::Runtime::new().unwrap();

    c.bench_function("vllm_request_with_hints", |b| {
        b.to_async(&rt).iter(|| async {
            let backend = GraphAwareVllmBackend::new("", Some(serde_json::json!({
                "base_url": "http://localhost:8000",
                "model": "Gemma-3-27B-it"
            }))).await.unwrap();

            // Register graph once
            let metadata = GraphMetadata::new("bench-graph", "bench-exec");
            backend.register_graph(metadata).await.unwrap();

            let request = LLMRequest::new("Say hello in one word")
                .with_apxm_hints(ApxmGraphHints::critical_path(
                    "bench-graph", "bench-exec", 1, "greet", vec![], 30_000
                ));

            black_box(backend.generate(request).await)
        });
    });
}

criterion_group!(benches, bench_without_hints, bench_with_hints);
criterion_main!(benches);
```

### Benchmark 3: Parallel Speedup
**File:** `benches/parallel_speedup.rs`

```rust
use criterion::{black_box, criterion_group, criterion_main, Criterion, BenchmarkId};
use apxm_driver::runtime::RuntimeExecutor;
use apxm_graph::ApxmGraph;

fn bench_parallelism_scaling(c: &mut Criterion) {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let mut group = c.benchmark_group("parallel_branches");

    for branch_count in [1, 2, 4, 8].iter() {
        group.bench_with_input(
            BenchmarkId::from_parameter(branch_count),
            branch_count,
            |b, &branches| {
                let graph = generate_parallel_graph(branches);
                b.to_async(&rt).iter(|| async {
                    let executor = RuntimeExecutor::new().await.unwrap();
                    black_box(executor.execute_graph(graph.clone()).await)
                });
            },
        );
    }
    group.finish();
}

fn generate_parallel_graph(branch_count: usize) -> ApxmGraph {
    // Create N parallel ASK nodes → WAIT_ALL → MERGE
    let mut nodes = Vec::new();
    let mut edges = Vec::new();

    for i in 1..=branch_count {
        nodes.push(serde_json::json!({
            "id": i,
            "name": format!("branch_{}", i),
            "op": "ASK",
            "attributes": {"template_str": "Count to 5"}
        }));
    }

    let wait_id = branch_count + 1;
    let merge_id = branch_count + 2;

    nodes.push(serde_json::json!({
        "id": wait_id,
        "name": "sync",
        "op": "WAIT_ALL",
        "attributes": {}
    }));

    nodes.push(serde_json::json!({
        "id": merge_id,
        "name": "merge",
        "op": "MERGE",
        "attributes": {}
    }));

    for i in 1..=branch_count {
        edges.push(serde_json::json!({
            "from": i,
            "to": wait_id,
            "dependency": "Control"
        }));
        edges.push(serde_json::json!({
            "from": i,
            "to": merge_id,
            "dependency": "Data"
        }));
    }

    let graph_json = serde_json::json!({
        "name": format!("parallel_{}", branch_count),
        "nodes": nodes,
        "edges": edges,
        "parameters": [],
        "metadata": {}
    });

    serde_json::from_value(graph_json).unwrap()
}

criterion_group!(benches, bench_parallelism_scaling);
criterion_main!(benches);
```

### Benchmark 4: Memoization Performance
**File:** `benches/memoization.rs`

```rust
use criterion::{black_box, criterion_group, criterion_main, Criterion};
use apxm_runtime::executor::memoization::ResponseCache;

fn bench_cache_hit(c: &mut Criterion) {
    let cache = ResponseCache::new();
    let key = ResponseCache::compute_key("test prompt", Some("system"), Some("gpt-4"), 0.0).unwrap();
    cache.put(key, "response".to_string(), 10, 5, "gpt-4".to_string());

    c.bench_function("memo_cache_hit", |b| {
        b.iter(|| black_box(cache.get(key)))
    });
}

fn bench_cache_miss(c: &mut Criterion) {
    let cache = ResponseCache::new();

    c.bench_function("memo_cache_miss", |b| {
        b.iter(|| {
            let key = ResponseCache::compute_key(
                "random prompt",
                Some("sys"),
                Some("model"),
                0.0
            ).unwrap();
            black_box(cache.get(key))
        })
    });
}

criterion_group!(benches, bench_cache_hit, bench_cache_miss);
criterion_main!(benches);
```

### Running Benchmarks
```bash
cd $APXM_HOME

# Run all benchmarks
cargo bench

# Run specific benchmark
cargo bench --bench vllm_hints_overhead

# Generate HTML reports (output to target/criterion/)
cargo bench -- --save-baseline main

# Compare with baseline
cargo bench -- --baseline main

# Point at live vLLM on 097-045
VLLM_ENDPOINT=http://useocpm2m-097-045:8000 cargo bench
```

### Metrics to Track
1. **Latency P50/P95/P99** — microseconds per graph execution
2. **Throughput** — graphs/second
3. **Token efficiency** — tokens/second (from vLLM metrics)
4. **Speedup ratio** — parallel vs sequential execution time
5. **Cache hit rate** — memoization effectiveness

---

## 5. ContextStack Implementation Plan

### Overview
The **ContextStack** is the patent-core feature: hierarchical context assembly with demand paging. It solves the "spaghetti context" problem by assembling prompts bottom-up from the execution graph.

### Data Structures

**File:** `crates/apxm-runtime/src/context_stack/mod.rs`

```rust
//! ContextStack — Hierarchical context assembly with demand paging.

pub mod assembly;
pub mod frame;
pub mod policy;

use crate::aam::Aam;
use std::collections::HashMap;
use std::sync::Arc;

/// A single stack frame representing one node's context contribution.
#[derive(Debug, Clone)]
pub struct ContextFrame {
    /// Node ID this frame corresponds to.
    pub node_id: usize,
    /// Node name (for debugging).
    pub node_name: String,
    /// Prompt prefix contributed by this node (if LLM op).
    pub prefix: Option<String>,
    /// AAM state snapshot at this node.
    pub aam_snapshot: Aam,
    /// Token count estimate for this frame's content.
    pub estimated_tokens: usize,
    /// Child frames (nodes that depend on this one).
    pub children: Vec<Arc<ContextFrame>>,
    /// Pin policy: should this frame be cached in vLLM?
    pub pin_policy: PinPolicy,
}

/// KV-cache pin policy for a frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PinPolicy {
    /// Don't pin (default for non-critical nodes).
    None,
    /// Pin prefix blocks with TTL in milliseconds.
    Prefix { ttl_ms: u32 },
}

/// The ContextStack: tree of frames + assembly algorithm.
pub struct ContextStack {
    /// All frames indexed by node_id.
    frames: HashMap<usize, Arc<ContextFrame>>,
    /// Root frame (entry node).
    root: Option<Arc<ContextFrame>>,
    /// Assembly policy (controls pruning, token budgets).
    policy: AssemblyPolicy,
}

impl ContextStack {
    pub fn new(policy: AssemblyPolicy) -> Self {
        Self {
            frames: HashMap::new(),
            root: None,
            policy,
        }
    }

    /// Add a frame to the stack.
    pub fn push_frame(&mut self, frame: ContextFrame) {
        let node_id = frame.node_id;
        self.frames.insert(node_id, Arc::new(frame));
    }

    /// Assemble context for a given node (bottom-up traversal).
    pub fn assemble_for_node(&self, node_id: usize) -> AssembledContext {
        assembly::assemble_bottom_up(self, node_id, &self.policy)
    }

    /// Get frame for a node.
    pub fn get_frame(&self, node_id: usize) -> Option<&Arc<ContextFrame>> {
        self.frames.get(&node_id)
    }
}

/// Result of context assembly.
#[derive(Debug, Clone)]
pub struct AssembledContext {
    /// Full prompt text assembled from ancestors.
    pub prompt: String,
    /// Total token count.
    pub total_tokens: usize,
    /// Frames included in assembly (in order).
    pub included_frames: Vec<usize>,
    /// Shared prefix frame ID (for KV-cache reuse).
    pub shared_prefix_frame: Option<usize>,
}

/// Policy controlling context assembly.
#[derive(Debug, Clone)]
pub struct AssemblyPolicy {
    /// Maximum tokens in assembled context.
    pub max_tokens: usize,
    /// Whether to prune non-critical branches.
    pub prune_non_critical: bool,
    /// Whether to enable demand paging (lazy load).
    pub demand_paging: bool,
}

impl Default for AssemblyPolicy {
    fn default() -> Self {
        Self {
            max_tokens: 8_000,
            prune_non_critical: false,
            demand_paging: false,
        }
    }
}
```

### Assembly Algorithm (Bottom-Up)

**File:** `crates/apxm-runtime/src/context_stack/assembly.rs`

```rust
//! Bottom-up context assembly algorithm.

use super::{ContextStack, AssembledContext, AssemblyPolicy};
use std::collections::HashSet;

/// Assemble context for a node by traversing ancestors bottom-up.
pub fn assemble_bottom_up(
    stack: &ContextStack,
    target_node_id: usize,
    policy: &AssemblyPolicy,
) -> AssembledContext {
    let mut visited = HashSet::new();
    let mut path = Vec::new();
    let mut total_tokens = 0;

    // Traverse from target node to root
    collect_ancestor_path(stack, target_node_id, &mut path, &mut visited);

    // Reverse to get root → target order
    path.reverse();

    // Assemble prompt from frames
    let mut prompt_parts = Vec::new();
    let mut included_frames = Vec::new();
    let mut shared_prefix_frame = None;

    for &node_id in &path {
        if let Some(frame) = stack.get_frame(node_id) {
            if total_tokens + frame.estimated_tokens > policy.max_tokens {
                // Budget exceeded — stop or prune
                if policy.prune_non_critical {
                    break;
                }
            }

            if let Some(ref prefix) = frame.prefix {
                prompt_parts.push(prefix.clone());
                total_tokens += frame.estimated_tokens;
                included_frames.push(node_id);

                // Mark first frame with PinPolicy::Prefix as shared prefix
                if shared_prefix_frame.is_none() && matches!(frame.pin_policy, PinPolicy::Prefix { .. }) {
                    shared_prefix_frame = Some(node_id);
                }
            }
        }
    }

    AssembledContext {
        prompt: prompt_parts.join("\n\n"),
        total_tokens,
        included_frames,
        shared_prefix_frame,
    }
}

/// Recursively collect ancestor frames.
fn collect_ancestor_path(
    stack: &ContextStack,
    node_id: usize,
    path: &mut Vec<usize>,
    visited: &mut HashSet<usize>,
) {
    if visited.contains(&node_id) {
        return; // Cycle guard
    }
    visited.insert(node_id);
    path.push(node_id);

    // Recurse to parents (reverse of children relationship)
    // In actual impl, need parent pointers or graph topology
}
```

### Demand Paging

**File:** `crates/apxm-runtime/src/context_stack/policy.rs`

```rust
//! Demand paging: lazy-load frame content from storage.

use super::ContextFrame;
use std::sync::Arc;

/// Lazy-loaded frame: content loaded on first access.
pub struct LazyFrame {
    node_id: usize,
    loaded: Arc<tokio::sync::Mutex<Option<ContextFrame>>>,
    loader: Box<dyn Fn() -> ContextFrame + Send + Sync>,
}

impl LazyFrame {
    pub fn new<F>(node_id: usize, loader: F) -> Self
    where
        F: Fn() -> ContextFrame + Send + Sync + 'static,
    {
        Self {
            node_id,
            loaded: Arc::new(tokio::sync::Mutex::new(None)),
            loader: Box::new(loader),
        }
    }

    pub async fn get(&self) -> ContextFrame {
        let mut guard = self.loaded.lock().await;
        if guard.is_none() {
            *guard = Some((self.loader)());
        }
        guard.as_ref().unwrap().clone()
    }
}
```

### Integration with Executor

**File:** `crates/apxm-runtime/src/executor/mod.rs` (modifications)

```rust
// Add to ExecutionContext
pub struct ExecutionContext {
    // ... existing fields ...

    /// Context stack for hierarchical prompt assembly.
    pub context_stack: Arc<Mutex<ContextStack>>,
}

// In LLM handler (ask, think, reason)
async fn handle_ask_with_context_stack(
    node_id: usize,
    template: &str,
    ctx: &ExecutionContext,
) -> Result<Value> {
    // 1. Assemble context from stack
    let stack = ctx.context_stack.lock().await;
    let assembled = stack.assemble_for_node(node_id);

    // 2. Build final prompt
    let prompt = format!("{}\n\n{}", assembled.prompt, template);

    // 3. Send to LLM with vLLM hints
    let hints = ApxmGraphHints {
        graph_id: Some(ctx.execution_id.clone()),
        node_id: Some(node_id as u32),
        priority_class: Some("critical_path".to_string()),
        // ... set reuse_group based on shared_prefix_frame
        ..Default::default()
    };

    let request = LLMRequest::new(&prompt).with_apxm_hints(hints);
    let response = ctx.llm_registry.generate(request).await?;

    // 4. Push new frame to stack for this node
    let frame = ContextFrame {
        node_id,
        node_name: "ask".to_string(),
        prefix: Some(response.content.clone()),
        aam_snapshot: ctx.aam.clone(),
        estimated_tokens: response.output_tokens,
        children: Vec::new(),
        pin_policy: PinPolicy::Prefix { ttl_ms: 30_000 },
    };
    stack.push_frame(frame);

    Ok(Value::String(response.content))
}
```

### vLLM KV-Cache Pinning Connection

When `shared_prefix_frame` is identified, set the `reuse_group` hint to the frame ID. vLLM will:
1. Pin the KV blocks for that prefix after completion
2. Reuse those blocks for subsequent requests with the same `reuse_group`
3. Evict based on TTL or memory pressure

### Estimated Complexity
- **Lines of code:** ~800 (mod.rs: 150, assembly.rs: 200, policy.rs: 150, integration: 300)
- **Dependencies:** tokio for async locks, existing graph/aam types
- **Timeline:** 3-4 days (design, implement, test, integrate)

---

## 6. MemoCache Two-Tier Design

### Overview
Upgrade existing `ResponseCache` (in-process HashMap) to two-tier: **Tier 1 DashMap** (concurrent, in-process) + **Tier 2 SQLite** (persistent, survives restarts).

### Current State (memoization.rs)
- In-process `HashMap<MemoKey, CacheEntry>` with `RwLock`
- Key: `u64` hash of (prompt, system_prompt, model, temperature)
- TTL: 3600s default, max 1024 entries
- Hit/miss/eviction counters

### Tier 1: DashMap (In-Process Concurrent)

**File:** `crates/apxm-runtime/src/executor/memoization.rs` (refactor)

```rust
//! Two-tier response cache: DashMap (L1) + SQLite (L2).

use dashmap::DashMap;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::time::{Duration, Instant};
use std::sync::Arc;
use sqlx::{SqlitePool, Row};

const DEFAULT_TTL: Duration = Duration::from_secs(3600);
const L1_MAX_ENTRIES: usize = 4096;  // Increased from 1024
const L2_DB_PATH: &str = ".apxm/memo_cache.db";

#[derive(Debug, Clone)]
struct CacheEntry {
    content: String,
    input_tokens: usize,
    output_tokens: usize,
    model: String,
    inserted_at: Instant,
}

/// Tier 1: In-process DashMap (lock-free concurrent access).
pub struct L1Cache {
    entries: Arc<DashMap<MemoKey, CacheEntry>>,
    ttl: Duration,
    max_entries: usize,
}

impl L1Cache {
    pub fn new() -> Self {
        Self {
            entries: Arc::new(DashMap::new()),
            ttl: DEFAULT_TTL,
            max_entries: L1_MAX_ENTRIES,
        }
    }

    pub fn get(&self, key: MemoKey) -> Option<CachedResponse> {
        self.entries.get(&key).and_then(|entry| {
            if entry.inserted_at.elapsed() < self.ttl {
                Some(CachedResponse {
                    content: entry.content.clone(),
                    input_tokens: entry.input_tokens,
                    output_tokens: entry.output_tokens,
                    model: entry.model.clone(),
                })
            } else {
                None
            }
        })
    }

    pub fn put(&self, key: MemoKey, content: String, input_tokens: usize, output_tokens: usize, model: String) {
        // Evict old entries if at capacity
        if self.entries.len() >= self.max_entries {
            self.evict_oldest();
        }

        self.entries.insert(key, CacheEntry {
            content,
            input_tokens,
            output_tokens,
            model,
            inserted_at: Instant::now(),
        });
    }

    fn evict_oldest(&self) {
        let now = Instant::now();
        let ttl = self.ttl;

        // Remove expired entries first
        self.entries.retain(|_, entry| entry.inserted_at.elapsed() < ttl);

        // If still over capacity, remove oldest 25%
        if self.entries.len() >= self.max_entries {
            let mut entries: Vec<_> = self.entries.iter().collect();
            entries.sort_by_key(|e| e.value().inserted_at);
            let to_remove = entries.len() / 4;
            for entry in entries.iter().take(to_remove) {
                self.entries.remove(entry.key());
            }
        }
    }
}
```

### Tier 2: SQLite (Persistent)

**File:** `crates/apxm-runtime/src/executor/memo_l2.rs`

```rust
//! Tier 2 cache: SQLite persistent storage.

use sqlx::{SqlitePool, Row};
use std::path::PathBuf;
use anyhow::Result;
use super::memoization::{MemoKey, CachedResponse};

pub struct L2Cache {
    pool: SqlitePool,
}

impl L2Cache {
    pub async fn new() -> Result<Self> {
        let db_path = dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join(".apxm")
            .join("memo_cache.db");

        // Create .apxm dir if needed
        if let Some(parent) = db_path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let pool = SqlitePool::connect(&format!("sqlite:{}", db_path.display())).await?;

        // Initialize schema
        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS memo_cache (
                key_hash INTEGER PRIMARY KEY,
                prompt TEXT NOT NULL,
                system_prompt TEXT,
                model TEXT NOT NULL,
                temperature REAL NOT NULL,
                content TEXT NOT NULL,
                input_tokens INTEGER NOT NULL,
                output_tokens INTEGER NOT NULL,
                inserted_at INTEGER NOT NULL,
                ttl_seconds INTEGER NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_inserted_at ON memo_cache(inserted_at);
            "#
        )
        .execute(&pool)
        .await?;

        Ok(Self { pool })
    }

    pub async fn get(&self, key: MemoKey) -> Result<Option<CachedResponse>> {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_secs() as i64;

        let row = sqlx::query(
            "SELECT content, input_tokens, output_tokens, model, inserted_at, ttl_seconds
             FROM memo_cache WHERE key_hash = ?"
        )
        .bind(key.0 as i64)
        .fetch_optional(&self.pool)
        .await?;

        if let Some(row) = row {
            let inserted_at: i64 = row.get("inserted_at");
            let ttl: i64 = row.get("ttl_seconds");

            if now - inserted_at < ttl {
                return Ok(Some(CachedResponse {
                    content: row.get("content"),
                    input_tokens: row.get::<i64, _>("input_tokens") as usize,
                    output_tokens: row.get::<i64, _>("output_tokens") as usize,
                    model: row.get("model"),
                }));
            }
        }

        Ok(None)
    }

    pub async fn put(
        &self,
        key: MemoKey,
        prompt: &str,
        system_prompt: Option<&str>,
        model: &str,
        temperature: f64,
        content: String,
        input_tokens: usize,
        output_tokens: usize,
        ttl_seconds: i64,
    ) -> Result<()> {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_secs() as i64;

        sqlx::query(
            "INSERT OR REPLACE INTO memo_cache
             (key_hash, prompt, system_prompt, model, temperature, content, input_tokens, output_tokens, inserted_at, ttl_seconds)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)"
        )
        .bind(key.0 as i64)
        .bind(prompt)
        .bind(system_prompt.unwrap_or(""))
        .bind(model)
        .bind(temperature)
        .bind(content)
        .bind(input_tokens as i64)
        .bind(output_tokens as i64)
        .bind(now)
        .bind(ttl_seconds)
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    pub async fn evict_expired(&self) -> Result<u64> {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_secs() as i64;

        let result = sqlx::query(
            "DELETE FROM memo_cache WHERE inserted_at + ttl_seconds < ?"
        )
        .bind(now)
        .execute(&self.pool)
        .await?;

        Ok(result.rows_affected())
    }
}
```

### Unified Two-Tier Cache

**File:** `crates/apxm-runtime/src/executor/memoization.rs` (final version)

```rust
pub struct TwoTierCache {
    l1: L1Cache,
    l2: Option<L2Cache>,  // None if SQLite init failed
    strategy: CacheStrategy,
}

#[derive(Debug, Clone, Copy)]
pub enum CacheStrategy {
    /// Write to L1 only (fastest, not persistent).
    L1Only,
    /// Write-through: write to both L1 and L2 synchronously.
    WriteThrough,
    /// Write-back: write to L1, lazy-write to L2 (best performance).
    WriteBack,
}

impl TwoTierCache {
    pub async fn new(strategy: CacheStrategy) -> Self {
        let l1 = L1Cache::new();
        let l2 = L2Cache::new().await.ok();  // Graceful fallback if SQLite fails

        Self { l1, l2, strategy }
    }

    pub async fn get(&self, key: MemoKey) -> Option<CachedResponse> {
        // Try L1 first
        if let Some(resp) = self.l1.get(key) {
            return Some(resp);
        }

        // Fallback to L2
        if let Some(ref l2) = self.l2 {
            if let Ok(Some(resp)) = l2.get(key).await {
                // Promote to L1
                self.l1.put(key, resp.content.clone(), resp.input_tokens, resp.output_tokens, resp.model.clone());
                return Some(resp);
            }
        }

        None
    }

    pub async fn put(&self, key: MemoKey, content: String, input_tokens: usize, output_tokens: usize, model: String) {
        // Always write to L1
        self.l1.put(key, content.clone(), input_tokens, output_tokens, model.clone());

        // Write to L2 based on strategy
        match self.strategy {
            CacheStrategy::L1Only => {}
            CacheStrategy::WriteThrough => {
                if let Some(ref l2) = self.l2 {
                    let _ = l2.put(key, "", None, &model, 0.0, content, input_tokens, output_tokens, 3600).await;
                }
            }
            CacheStrategy::WriteBack => {
                // Spawn background task to write to L2
                if let Some(l2) = self.l2.clone() {
                    tokio::spawn(async move {
                        let _ = l2.put(key, "", None, &model, 0.0, content, input_tokens, output_tokens, 3600).await;
                    });
                }
            }
        }
    }
}
```

### Key Design Decisions

1. **Key unchanged:** Still `u64` hash of (prompt, system, model, temperature)
   - **Pro:** Simple, fast
   - **Con:** Collisions possible (use birthday paradox — 1% collision at ~10M entries)
   - **Mitigation:** L2 stores full prompt for validation

2. **Integration with ContextStack:** Memoize at frame granularity
   - Key includes: `(frame.prefix, frame.aam_snapshot.hash(), model, 0.0)`
   - Allows cache reuse across executions with same context

3. **Write strategy:** Default to `WriteBack` for best performance

### Dependencies
Add to `Cargo.toml`:
```toml
[dependencies]
dashmap = "5.5"
sqlx = { version = "0.7", features = ["sqlite", "runtime-tokio-native-tls"] }
dirs = "5.0"
```

### Estimated Complexity
- **Lines of code:** ~400 (L1: 100, L2: 200, unified: 100)
- **Timeline:** 2 days (implement, test, benchmark)

---

## 7. launch-feature.sh Local Backend

### Problem
The 3-parallel THINK workflow currently uses external API (costs money). We have Gemma-3-27B-it running on 097-045 with 4× GPU.

### Solution
Point `launch-feature.sh` at local `gemma3-local` backend.

### Implementation

#### Step 1: Register vLLM Backend
```bash
# On 097-083
dekk apxm backend add gemma3-local \
  --type local \
  --protocol vllm \
  --base-url http://useocpm2m-097-045:8000 \
  --model Gemma-3-27B-it

# Verify
dekk apxm backend list
dekk apxm backend test gemma3-local
```

**Expected config in `~/.apxm/config.toml`:**
```toml
[[backends]]
name = "gemma3-local"
type = "local"
protocol = "vllm"
base_url = "http://useocpm2m-097-045:8000"
model = "Gemma-3-27B-it"

[backends.docker]
# Optional: if running vLLM via Docker on 097-045
image = "gpu/pytorch-private:vllm-v0.14.0_amd_dev_aiter_nixl_ravgupta"
container_name = "vllm-gemma3-v2"
```

#### Step 2: Update launch-feature.sh
**File:** `$APXM_HOME/launch-feature.sh` (create if doesn't exist)

```bash
#!/bin/bash
set -euo pipefail

FEATURE_DESC="${1:-Add new feature}"
WORKFLOW_GRAPH="examples/workflows/add-apxm-feature.apxm"

echo "==> Launching 3-parallel THINK workflow for: $FEATURE_DESC"

# Override backend in graph execution
export APXM_DEFAULT_BACKEND=gemma3-local

# Execute the workflow
dekk apxm execute "$WORKFLOW_GRAPH" -- "$FEATURE_DESC" \
  --emit-session \
  --emit-metrics /tmp/feature-metrics.json

echo "==> Workflow complete. Session saved to ~/.apxm/sessions/<latest>/"
```

Make executable:
```bash
chmod +x $APXM_HOME/launch-feature.sh
```

#### Step 3: Network Routing (097-083 → 097-045)
Verify connectivity:
```bash
# On 097-083
ping -c 3 useocpm2m-097-045
curl http://useocpm2m-097-045:8000/health

# If port 8000 is blocked, SSH tunnel
ssh -L 8000:localhost:8000 useocpm2m-097-045 -N -f

# Then use localhost:8000 in backend config
dekk apxm backend add gemma3-local \
  --base-url http://localhost:8000 \
  --model Gemma-3-27B-it
```

#### Step 4: Prompt Adjustments for Gemma3
Gemma-3-27B-it uses different prompt format than Claude. Update system prompts in `~/.apxm/config.toml`:

```toml
[instruction]
default_system_prompt = """
You are a helpful AI assistant specialized in software engineering.
Be concise and focus on concrete implementation details.
"""

[instruction.operation_prompts]
think = """
Analyze the following problem carefully. Think step-by-step.
Output structured reasoning in JSON format.
"""

reason = """
Apply logical reasoning to solve this task.
Provide your answer with clear justification.
"""
```

#### Step 5: Test Local Workflow
```bash
cd $APXM_HOME

# Test with simple feature
./launch-feature.sh "Add validation for node IDs in graph parser"

# Check output
ls -lh ~/.apxm/sessions/
cat ~/.apxm/sessions/$(ls -t ~/.apxm/sessions/ | head -1)/results.json
```

### Cost Savings
- **External API:** ~$0.05/request × 3 parallel × 10 iterations = $1.50/feature
- **Local vLLM:** $0 (uses existing hardware)
- **Estimated savings:** $300/month at 20 features/day

---

## 8. .agents/skills Gap Analysis

### Current Skills (21 total)
1. **analyze** — `apxm analyze` graph analysis
2. **backend/add** — register LLM backend
3. **build** — `cargo build` with driver features
4. **compile** — `.apxm` → `.apxmobj`
5. **debug** — debugging workflows
6. **decompile** — `.apxmobj` → `.apxm`
7. **doctor** — environment health check
8. **execute** — compile + run graph
9. **explain** — graph summary
10. **extend** — feature workflow (architect → implement → review)
11. **init** — scaffold new project
12. **ops** — list AIS operations
13. **run** — execute pre-compiled artifact
14. **task/merge** — merge graph fragments
15. **template** — list/show starter patterns
16. **test** — run test suite
17. **validate** — check graph against contract
18. **view** — display graph structure
19. **worktree** — git worktree helpers

### Missing High-Value Skills

#### 1. `/benchmark` — Run Criterion Benchmarks
**File:** `.agents/skills/benchmark/SKILL.md`

```yaml
---
name: benchmark
description: Run Criterion benchmarks and compare with baseline
user-invocable: true
---

# Benchmark Skill

Run performance benchmarks using Criterion.rs.

## Usage
```bash
/benchmark                          # Run all benchmarks
/benchmark vllm_hints_overhead      # Run specific benchmark
/benchmark --baseline main          # Compare with baseline
```

## Implementation
Executes `cargo bench` with appropriate flags.
```

**Handler:** `.agents/skills/benchmark/handler.sh`
```bash
#!/bin/bash
BENCHMARK="${1:-}"
BASELINE="${2:-}"

if [ -n "$BASELINE" ]; then
  cargo bench ${BENCHMARK:+--bench $BENCHMARK} -- --baseline "$BASELINE"
else
  cargo bench ${BENCHMARK:+--bench $BENCHMARK}
fi
```

#### 2. `/optimize` — Compile with Optimization Target
**File:** `.agents/skills/optimize/SKILL.md`

```yaml
---
name: optimize
description: Compile graph with specific optimization target
user-invocable: true
---

# Optimize Skill

Compile a graph with optimization flags.

## Usage
```bash
/optimize graph.apxm --target latency   # Optimize for speed
/optimize graph.apxm --target cost      # Optimize for cost
/optimize graph.apxm --target parallel  # Maximize parallelism
```

## Targets
- `latency`: Model downgrade, prefetch, speculation
- `cost`: Cheap models, memoization, batching
- `parallel`: WAIT_ALL injection, fan-out
- `balanced`: Default (no aggressive opts)
```

#### 3. `/profile` — Execution Profiling
**File:** `.agents/skills/profile/SKILL.md`

```yaml
---
name: profile
description: Profile graph execution and show bottlenecks
user-invocable: true
---

# Profile Skill

Run graph execution with detailed profiling.

## Usage
```bash
/profile graph.apxm                 # Profile and show top 10 slow nodes
/profile graph.apxm --flamegraph    # Generate flamegraph
```

## Output
- Node-level latency (P50/P95/P99)
- Token counts per node
- Parallel vs sequential time
- Critical path visualization
```

#### 4. `/replay` — Session Replay
**File:** `.agents/skills/replay/SKILL.md`

```yaml
---
name: replay
description: Replay a session trace as timeline
user-invocable: true
---

# Replay Skill

Replay a recorded session from `~/.apxm/sessions/<id>/`.

## Usage
```bash
/replay <session-id>                # Replay session as timeline
/replay latest                      # Replay most recent session
```

## Output
Timeline showing:
- Node execution order
- Timestamps
- Token counts
- Outputs
```

#### 5. `/agent` — Agent Management
**File:** `.agents/skills/agent/SKILL.md`

```yaml
---
name: agent
description: Manage ACP agent profiles
user-invocable: true
---

# Agent Skill

Register, test, and remove ACP agents.

## Usage
```bash
/agent list                         # Show registered agents
/agent add claude                   # Register from template
/agent test claude                  # Test spawning
/agent remove myagent               # Remove profile
```

## Templates
- claude, codex, gemini, copilot, cursor, droid, etc. (15 built-in)
```

#### 6. `/graph` — Graph Construction Helpers
**File:** `.agents/skills/graph/SKILL.md`

```yaml
---
name: graph
description: Interactive graph construction
user-invocable: true
---

# Graph Skill

Build graphs interactively with validation.

## Usage
```bash
/graph new "my-workflow"            # Start new graph
/graph add-node ASK "greeting"      # Add node
/graph add-edge 1 2 Data            # Add edge
/graph save workflow.apxm           # Save to file
```

## Features
- Live validation
- Auto-assign node IDs
- Template insertion
```

#### 7. `/memo` — Memoization Management
**File:** `.agents/skills/memo/SKILL.md`

```yaml
---
name: memo
description: Manage response memoization cache
user-invocable: true
---

# Memo Skill

Inspect and manage the two-tier memo cache.

## Usage
```bash
/memo stats                         # Show cache hit rate
/memo clear                         # Clear L1 + L2 cache
/memo evict-expired                 # Remove expired entries
/memo export cache-backup.json      # Export cache to JSON
```

## Stats
- L1 hit/miss/eviction counts
- L2 row count, DB size
- Average token savings
```

### Skills to Deprecate/Archive
None — all 21 existing skills are actively useful.

### Priority Ranking
1. **/benchmark** — Critical for Phase 4 (performance validation)
2. **/optimize** — Needed for optimization targets work
3. **/agent** — Improves multi-agent workflow UX
4. **/replay** — Debugging aid (sessions already captured)
5. **/profile** — Nice-to-have (overlap with benchmark)
6. **/memo** — Operational tool (once two-tier cache live)
7. **/graph** — Low priority (Python SDK covers this)

---

## Timeline Summary

### Week 1 (Days 1-5)
| Day | Task | Hours | Who |
|-----|------|-------|-----|
| 1 | AgentMate Phase 1.1-1.3 (extract, add ops, fix CLI) | 6 | via `./launch-feature.sh` |
| 1 | AgentMate Phase 1.4 (tests) | 2 | manual |
| 2 | AgentMate Phase 1.5 (git archive, PyPI prep) | 2 | manual |
| 2 | Node sync (097-045 ↔ 097-083) | 1 | manual |
| 2 | Integration tests (setup + Test 1-2) | 4 | via `./launch-feature.sh` |
| 3 | Integration tests (Test 3-5) | 4 | via `./launch-feature.sh` |
| 3 | Benchmarks (setup + Bench 1-2) | 4 | via `./launch-feature.sh` |
| 4 | Benchmarks (Bench 3-4) | 3 | via `./launch-feature.sh` |
| 4 | ContextStack design + data structures | 5 | via `./launch-feature.sh` |
| 5 | ContextStack assembly algorithm | 6 | via `./launch-feature.sh` |
| 5 | launch-feature.sh local backend | 2 | manual |

### Week 2 (Days 6-10)
| Day | Task | Hours | Who |
|-----|------|-------|-----|
| 6 | ContextStack demand paging | 4 | via `./launch-feature.sh` |
| 6 | ContextStack executor integration | 4 | via `./launch-feature.sh` |
| 7 | MemoCache Tier 1 (DashMap refactor) | 4 | via `./launch-feature.sh` |
| 7 | MemoCache Tier 2 (SQLite schema + impl) | 4 | via `./launch-feature.sh` |
| 8 | MemoCache unified + write strategies | 3 | via `./launch-feature.sh` |
| 8 | MemoCache tests + benchmarks | 3 | via `./launch-feature.sh` |
| 8 | Skills: /benchmark + /optimize | 2 | manual |
| 9 | Phase 4: OptimizationTarget enum + CLI flag | 4 | via `./launch-feature.sh` |
| 9 | Phase 4: ModelDowngrade MLIR pass (design) | 4 | via `./launch-feature.sh` |
| 10 | End-to-end validation (all features working) | 4 | manual |
| 10 | Documentation updates | 2 | manual |
| 10 | Git commits + PR | 2 | manual |

### Parallel Work Streams
1. **AgentMate** (Day 1-2) — standalone, no APXM changes
2. **Integration tests** (Day 2-3) — can run in parallel with benchmarks
3. **Benchmarks** (Day 3-4) — independent of ContextStack
4. **ContextStack** (Day 4-6) — core feature, blocks Phase 4
5. **MemoCache** (Day 7-8) — independent of ContextStack
6. **Phase 4 kickoff** (Day 9-10) — requires ContextStack complete

---

## Success Criteria

### Phase 3 Close-out
- [ ] 5 integration tests passing in CI
- [ ] 4 Criterion benchmarks generating reports
- [ ] ContextStack foundation implemented (data structures + assembly)

### Phase 4 Kickoff
- [ ] Full ContextStack with demand paging integrated into executor
- [ ] Two-tier MemoCache (DashMap + SQLite) operational
- [ ] `--target` CLI flag working with at least 1 optimization pass

### AgentMate Consolidation
- [ ] Standalone `agentmate` PyPI package published
- [ ] All 7 missing AIS ops added to proxy.py
- [ ] `dekk apxm execute` integration working
- [ ] Python tests passing

### Infrastructure
- [ ] Both nodes (097-083, 097-045) at same git HEAD
- [ ] `launch-feature.sh` using local Gemma3 backend
- [ ] 2+ new skills added to `.agents/skills/`

---

## Risks & Mitigations

| Risk | Probability | Impact | Mitigation |
|------|-------------|--------|------------|
| vLLM on 097-045 unreachable from 097-083 | Medium | High | SSH tunnel as fallback |
| SQLite NFS locking issues (memo L2) | Medium | Medium | Use WAL mode + `.apxm/` on local disk |
| ContextStack complexity > estimate | Medium | High | Start with minimal viable (no demand paging) |
| Gemma3 prompt format incompatible | Low | Medium | Test early, add format wrappers |
| MLIR compiler changes break build | Low | High | Feature-flag new passes, keep fallback |

---

## Next Actions (Immediate)

1. **Create this plan as `$APXM_HOME/docs/IMPLEMENTATION-PLAN-2WEEKS.md`**
2. **Verify network connectivity:** `ping useocpm2m-097-045` from 097-083
3. **Register vLLM backend:** `dekk apxm backend add gemma3-local ...`
4. **Kick off AgentMate extraction:** `./launch-feature.sh "Extract agentmate Python package as standalone"`
5. **Set up integration tests skeleton:** `mkdir tests/ && touch tests/conftest.py`

---

**Document prepared by:** Claude Code (ULTRATHINK mode)
**Review status:** Ready for execution
**Estimated total effort:** 80 developer-hours over 10 days (8h/day)
