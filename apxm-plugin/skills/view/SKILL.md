---
name: view
description: Open the interactive graph visualizer in a browser
user-invocable: true
---

# View

Launches an interactive, web-based graph visualizer for exploring APXM workflow graphs. Opens in your browser with a dark-themed canvas showing nodes colored by AIS category, edges styled by dependency type, and a detail inspector panel. Useful for understanding graph structure, verifying connections, and exploring complex workflows visually before compiling.

The visualizer is a React + TypeScript app using ReactFlow for rendering and ELK (Eclipse Layout Kernel) for automatic hierarchical DAG layout. It fetches AIS operation metadata from the CLI at runtime, so new operations appear automatically without viewer changes.

## Commands

```bash
dekk apxm view graph.apxm              # open graph in browser
dekk apxm view graph.apxm --no-open    # start server without opening browser (navigate to http://127.0.0.1:4174)
```

## Interactive Features

**Canvas:**
- Pan (click-drag), zoom (scroll/pinch), fit-to-view button
- Toggle layout direction: top-to-bottom (default) or left-to-right
- Drag-and-drop an `.apxm` file onto the canvas to load a different graph
- Open file picker button to browse for graphs

**Node cards** show:
- Category badge (e.g., REASONING, TOOLS, MEMORY) with category-specific color
- Operation badge (ASK, THINK, INV, etc.)
- Entry/exit markers for nodes with no incoming or outgoing edges
- Latency tier pill (LOW / MED / HIGH)
- Node name and ID
- First 2 attributes, truncated
- Incoming/outgoing edge counts

**Node inspector** (right panel, appears on click):
- Full operation details: category, latency, description
- All attributes as key-value pairs
- Connections: lists incoming and outgoing edges with dependency types
- Collapsible raw JSON view of the node

**Legend** (bottom-left, collapsible):
- 10 AIS categories with color swatches (Reasoning, Tools, Memory, Sync, Control Flow, Error Handling, Communication, Coordination, Identity, Internal)
- 3 edge types: Data (solid blue), Control (dashed red), Effect (dotted gold)

## Architecture

- **Server**: Vite dev server on `http://127.0.0.1:4174` with API middleware
- **API endpoints**: `GET /api/graph` (returns the loaded graph JSON), `GET /api/ops` (returns AIS op metadata from the CLI)
- **Layout**: ELK layered algorithm with crossing minimization, 56px node spacing, 96px layer spacing
- **Auto-install**: npm dependencies are installed automatically on first run if `node_modules/` is missing

## When to Use

- After authoring or editing a graph, to visually verify structure and connections
- When exploring a complex workflow with many parallel branches
- To understand data flow and dependency patterns in unfamiliar graphs
- Before compilation, alongside `dekk apxm validate` and `dekk apxm analyze`
