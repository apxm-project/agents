# The shared frontend graph model (WF-3)

`ApxmGraph` (defined in [`apxm/ir.py`](../apxm/ir.py)) is **the shared
frontend-internal graph model**. It is produced by
`GraphRecorder.to_graph()` ([`apxm/proxy.py`](../apxm/proxy.py)) — the
in-memory result of recording a Python-authored flow (`@compile()` /
`g.<op>()` calls) before it is lowered to `.air` text.

Python is the **reference emitter**: TypeScript's `@apxm/frontend` package
(`crates/compiler/frontend/typescript/src/graph.ts`) mirrors this exact
shape and is checked against it for structural parity. This document is
that contract, written down once so both frontends (and anyone adding a
third) can be verified against it without reading two implementations
side by side.

## What this is not

`ApxmGraph` is **not** a server wire format. The only artifact that
crosses the frontend/runtime boundary is the `.air` MLIR text produced by
`to_air()` — see [`air-grammar.md`](../../../../../contracts/docs/air-grammar.md)
in the coordinator's contracts repo for that grammar. `ApxmGraph` and its
plain-dict `to_dict()` form exist purely as frontend-internal tooling
surface: something typed and serializable for tests, graph transforms
(`ApxmGraph.merge()`), and pre-emission validation
(`apxm.ir.validate_against_apxm`) to work against.

## Shape

```text
ApxmGraph
  name: str
  nodes: [GraphNode]
  edges: [GraphEdge]
  parameters: [Parameter]
  metadata: dict[str, Any]

GraphNode
  id: int            # positive, unique within the graph, assigned in recording order
  name: str           # SSA identifier stem; emitted as `%<name>`
  op: str             # upper-case AIS op name, e.g. "ASK", "SPAWN_AGENT"
  attributes: dict[str, Any]   # op-specific, JSON-plain attribute bag

GraphEdge
  from_id: int        # source node id       -- serializes as "from"
  to_id: int          # destination node id   -- serializes as "to"
  dependency: str     # "Data" | "Control" | "Effect"

Parameter
  name: str
  type_name: str      # "str" | "int" | "float" | "bool" | "json" (or a non-standard value, warned)
```

### `ApxmGraph.to_dict()` — the canonical plain-dict serialization

```json
{
  "name": "my_flow",
  "nodes": [
    {"id": 1, "name": "greet", "op": "ASK", "attributes": {"template_str": "Greet {name}."}}
  ],
  "edges": [
    {"from": 1, "to": 2, "dependency": "Data"}
  ],
  "parameters": [
    {"name": "name", "type_name": "str"}
  ],
  "metadata": {"is_entry": true}
}
```

Every value is JSON-plain (str, int, float, bool, list, dict, or `None`) —
`json.dumps(graph.to_dict())` round-trips through `ApxmGraph.from_dict()`
unchanged. This is the exact shape a second frontend's serialization must
match field-for-field: same top-level keys (`name`, `nodes`, `edges`,
`parameters`, `metadata`), same nested field names (`id`, `name`, `op`,
`attributes`; `from`, `to`, `dependency`; `name`, `type_name`).

TypeScript's `ApxmGraph.toDict()` produces this same shape (see
`typescript/src/graph.ts`): its `GraphEdge.from`/`GraphEdge.to` fields and
`Parameter.typeName` → `type_name` dict key already match on the wire; only
the in-memory TS field name (`typeName`, camelCase per TS convention)
differs from Python's `type_name` attribute name — the *serialized* key is
identical.

## Field semantics

- **`nodes` order** is recording order, not execution order. `to_air()`
  topologically sorts by `edges` before emitting; TS's emitter does the
  same (`topologicalSort` in both `apxm/utils.py` and
  `typescript/src/utils.ts`).
- **`edges[].dependency`** — only `"Data"` edges become SSA operands in
  emitted MLIR (an op's declared inputs are its incoming Data edges, in
  edge order). `"Control"` and `"Effect"` edges affect topological
  ordering only and are invisible in the emitted text otherwise.
- **`metadata.is_entry`** (bool, or a truthy string —
  `"true"`/`"1"`/`"yes"`, case-insensitive) — the only metadata key
  `to_air()` currently interprets: when true (the default when a
  `GraphRecorder` is constructed without explicit `metadata`), the emitted
  function carries `attributes {ais.entry}`. Every other metadata key is
  preserved through `to_dict()`/`from_dict()` but not otherwise
  interpreted by `ApxmGraph`.
- **`GraphNode.attributes`** values must already be JSON-plain by the time
  they reach `ApxmGraph` — `GraphRecorder`'s op methods (`g.ask()`,
  `g.spawn_agent()`, ...) normalize Python-native inputs (dicts to JSON
  strings where the op expects a JSON blob, enums to their string value,
  etc.) before storing them. `_generated/emission.py`'s per-op emitters
  decide which attributes become MLIR positional operands, `to "..."`
  syntax, or `{key = value}` keyword attributes; an attribute an emitter
  doesn't reference is silently dropped from the emitted text (it is not
  an error — several structural ops, e.g. `PAUSE`/`RESUME`/`FENCE`,
  currently emit no attributes at all beyond the bare op and its data
  operands).

## Producing this shape

```python
from apxm import GraphRecorder

g = GraphRecorder("my_flow")
greeting = g.ask(name="greet", prompt="Greet {name} warmly.")
g.done(greeting)

graph = g.to_graph()        # -> ApxmGraph
air = graph.to_air()        # -> ".air" MLIR text
# or, equivalently and including the python-tools sidecar comment:
air = g.to_air()
```

`GraphRecorder.to_graph()` validates every recorded backend route
(`apxm.backends.validate_graph_routes`) and returns a defensive copy: the
`ApxmGraph` it returns is independent of further mutations to the
recorder. `to_air()` is exactly `to_graph().to_air()` plus, when Python
`@tool`-decorated functions were registered, a leading
`// apxm:python-tools ...` sidecar comment carrying their manifest.

## Cross-frontend parity vectors

The `.air` output both frontends must independently produce
grammar-valid text for lives in the coordinator's
`workspace/contracts/vectors/air/` vector set (WF-2, hardened under WF-3).
See that directory's `manifest.json` for the fixture list and
[`air-grammar.md`](../../../../../contracts/docs/air-grammar.md) for the
grammar itself. `tools/check_air_provenance.py` in the coordinator repo is
the reusable grammar check; both frontends' test suites use it (or an
equivalent structural check) against graphs built to the shape documented
above.
