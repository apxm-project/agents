---
name: gui
description: Open the APXM GUI in a browser
user-invocable: true
---

# GUI

Launches the APXM GUI for exploring APXM workflow graphs. Opens in your browser with a canvas showing nodes by AIS category, dependency-typed edges, and detail panels for source, graph, and runtime state.

The visualizer is a React + TypeScript app using ReactFlow for rendering and ELK (Eclipse Layout Kernel) for automatic hierarchical DAG layout. It fetches AIS operation metadata from the CLI at runtime, so new operations appear automatically without viewer changes.

## Commands

```bash
dekk apxm gui graph.air --open        # open graph in browser
dekk apxm gui graph.air --port 18801  # start server without opening browser
```

## Interactive Features

**Canvas:**
- Pan (click-drag), zoom (scroll/pinch), fit-to-view button
- Toggle layout direction: top-to-bottom (default) or left-to-right
- Drag-and-drop an `.air` file onto the canvas to load a different graph
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

- **Server**: Axum server with embedded React assets
- **API endpoints**: graph APIs return parsed graph data derived from AIR; operation APIs return AIS metadata from the CLI
- **Layout**: ELK layered algorithm with crossing minimization, 56px node spacing, 96px layer spacing
- **Auto-install**: npm dependencies are installed automatically on first run if `node_modules/` is missing

## When to Use

- After authoring or editing a graph, to visually verify structure and connections
- When exploring a complex workflow with many parallel branches
- To understand data flow and dependency patterns in unfamiliar graphs
- Before compilation, alongside `dekk apxm validate` and `dekk apxm analyze`
