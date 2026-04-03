# Create Graph Workflow

Build an APXM graph from scratch. Input: **$ARGUMENTS** (describe the workflow goal, e.g., `research assistant that summarizes and critiques`)

---

## Step 1: Discover available operations

```bash
dekk apxm ops list --json
```

Review the operations and identify which ones are needed for the user's workflow goal.

## Step 2: Pick a starter template

```bash
dekk apxm template list
```

If a template matches the workflow pattern, fetch it:

```bash
dekk apxm template show <template-name> --json
```

Use this as the starting skeleton. If no template fits, start from the minimal graph structure:

```json
{
  "name": "...",
  "nodes": [],
  "edges": [],
  "parameters": [],
  "metadata": {}
}
```

## Step 3: Design nodes

For each step in the workflow, create a node with the appropriate AIS operation. Look up op details with:

```bash
dekk apxm ops show <OP_NAME>
```

Each node needs: `id` (unique int), `name` (descriptive), `op` (AIS operation name), `attributes` (op-specific).

## Step 4: Design edges

Connect nodes with edges. Each edge has:
- `from`: source node id
- `to`: target node id
- `dependency`: one of `Data`, `Control`, `Effect`

Use `Data` for value flow, `Control` for ordering, `Effect` for side-effect ordering.

## Step 5: Write the graph file

Write the complete graph JSON to the user's chosen path (default: `flows/<name>.json`).

## Step 6: Validate

```bash
dekk apxm validate <file>
```

Fix any validation errors before proceeding.

## Step 7: Analyze

```bash
dekk apxm analyze <file>
```

Review parallelism opportunities, critical path, and estimated speedup.

## Step 8: Explain

```bash
dekk apxm explain <file>
```

Confirm the human-readable summary matches the user's intent. Iterate if needed.
