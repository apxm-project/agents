# Your First Graph

Build APXM graphs from scratch using JSON and the CLI. Each example is copy-pasteable and builds on the previous one.

---

## 1. Single Node

The smallest valid graph: one ASK node, no edges.

Create `hello.apxm`:

```json
{
  "name": "hello",
  "nodes": [
    {"id": 1, "name": "greet", "op": "ASK", "attributes": {"template_str": "Explain quantum computing in one sentence"}}
  ],
  "edges": [],
  "parameters": [],
  "metadata": {}
}
```

Every graph requires five fields: `name`, `nodes`, `edges`, `parameters`, and `metadata`. See the [Graph Format Reference](../reference/graph-format.md) for the full schema.

Validate and inspect:

```bash
apxm validate hello.apxm
apxm explain hello.apxm
```

`validate` checks the graph against AIS operation contracts. `explain` prints a human-readable summary of what the graph does.

---

## 2. Pipeline

Chain two nodes with a `Data` edge. The second node references the first node's output via `{{node_1}}`.

Create `pipeline.apxm`:

```json
{
  "name": "pipeline",
  "nodes": [
    {"id": 1, "name": "draft", "op": "ASK", "attributes": {"template_str": "Write a haiku about Rust"}},
    {"id": 2, "name": "critique", "op": "ASK", "attributes": {"template_str": "Critique this haiku and suggest improvements: {{node_1}}"}}
  ],
  "edges": [
    {"from": 1, "to": 2, "dependency": "Data"}
  ],
  "parameters": [],
  "metadata": {}
}
```

The `Data` edge tells the scheduler that node 2 cannot start until node 1 finishes. At runtime, `{{node_1}}` resolves to node 1's output.

Analyze the execution plan:

```bash
apxm analyze pipeline.apxm
```

This reports two sequential phases: node 1 runs first, then node 2.

---

## 3. Fan-Out

Two independent ASK nodes run in parallel. A WAIT_ALL node synchronizes them.

Create `fanout.apxm`:

```json
{
  "name": "fan-out",
  "nodes": [
    {"id": 1, "name": "pros", "op": "ASK", "attributes": {"template_str": "List 3 pros of static typing"}},
    {"id": 2, "name": "cons", "op": "ASK", "attributes": {"template_str": "List 3 cons of static typing"}},
    {"id": 3, "name": "sync", "op": "WAIT_ALL", "attributes": {"tokens": ["{{node_1}}", "{{node_2}}"]}}
  ],
  "edges": [
    {"from": 1, "to": 3, "dependency": "Data"},
    {"from": 2, "to": 3, "dependency": "Data"}
  ],
  "parameters": [],
  "metadata": {}
}
```

Nodes 1 and 2 share no edges, so the scheduler runs them concurrently. Node 3 waits for both.

Confirm parallelism:

```bash
apxm analyze fanout.apxm
```

The output places nodes 1 and 2 in the same execution phase and reports a speedup estimate.

---

## 4. Parameters

Pass values into a graph at runtime. Add a `parameters` array with `name` and `type_name` entries. Use `{{param_name}}` in templates to reference them.

Create `parameterized.apxm`:

```json
{
  "name": "parameterized",
  "nodes": [
    {"id": 1, "name": "research", "op": "ASK", "attributes": {"template_str": "Summarize the key ideas of {{topic}}"}},
    {"id": 2, "name": "quiz", "op": "ASK", "attributes": {"template_str": "Write 3 quiz questions about {{topic}} based on: {{node_1}}"}}
  ],
  "edges": [
    {"from": 1, "to": 2, "dependency": "Data"}
  ],
  "parameters": [
    {"name": "topic", "type_name": "str"}
  ],
  "metadata": {}
}
```

Valid `type_name` values: `str`, `int`, `float`, `bool`, `json`.

Validate and run with a parameter value:

```bash
apxm validate parameterized.apxm
apxm execute parameterized.apxm "quantum computing"
```

---

## 5. Compile and Execute

Separate compilation from execution for production use. Artifacts are portable binaries -- ship them without source code.

Compile to an optimized artifact:

```bash
apxm compile fanout.apxm -o fanout.apxmobj
```

Run a pre-compiled artifact (skips compilation):

```bash
apxm run fanout.apxmobj
```

Compile and execute in one step:

```bash
apxm execute fanout.apxm
```

Control the optimization level with `-O`:

```bash
apxm execute fanout.apxm -O2    # full optimization (FuseReasoning, PromptCanonicalization, etc.)
apxm execute fanout.apxm -O0    # no optimization
```

See [Optimization Overview](../optimization/overview.md) for what each pass does and when `-O2` helps.

Emit execution metrics for analysis:

```bash
apxm execute fanout.apxm --emit-metrics metrics.json
apxm execute fanout.apxm --emit-session    # full session trace
```

---

## 6. Composing Graphs

Merge graph fragments into a single workflow with `apxm task merge`.

`step-a.apxm`:

```json
{
  "name": "step-a",
  "nodes": [
    {"id": 1, "name": "research", "op": "ASK", "attributes": {"template_str": "Research the history of LLVM"}}
  ],
  "edges": [],
  "parameters": [],
  "metadata": {}
}
```

`step-b.apxm`:

```json
{
  "name": "step-b",
  "nodes": [
    {"id": 1, "name": "research", "op": "ASK", "attributes": {"template_str": "Research the history of GCC"}}
  ],
  "edges": [],
  "parameters": [],
  "metadata": {}
}
```

Merge them:

```bash
apxm task merge step-a.apxm step-b.apxm --name combined -o combined.apxm
```

This re-numbers node IDs to avoid collisions and adds a WAIT_ALL sync node. Validate the result:

```bash
apxm validate combined.apxm
apxm analyze combined.apxm
```

---

## Edge Types

Three dependency types control scheduling:

| Type | Meaning |
|------|---------|
| `Data` | Output of source flows into target's template placeholders |
| `Control` | Source must complete before target starts (no data transfer) |
| `Effect` | Source produces a side effect that target depends on |

---

## Next Steps

- **Browse operations** -- `apxm ops list` shows all 39 AIS operations; `apxm ops show ASK` prints details and example JSON
- **Use templates** -- `apxm template list` lists built-in patterns (pipeline, fan-out, map-reduce, conditional); `apxm template show fan-out --json` emits ready-to-use graph JSON
- **Configure backends** -- [Backend Setup](../guides/backends.md) for providers, model routing, and rate limits
- **Optimize** -- [Optimization Overview](../optimization/overview.md) for compiler passes and performance tuning
- **Integrate** -- [vLLM](../integrations/vllm.md) and [DSPy](../integrations/dspy.md) for production deployments
- **Reference** -- [Config Reference](../reference/config.md) for `~/.apxm/config.toml`, [Graph Format](../reference/graph-format.md) for the full `.apxm` JSON schema
