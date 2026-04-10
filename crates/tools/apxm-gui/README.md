# apxm-gui

Web-based visualization for agent workflow graphs, compiler optimizations, and session traces.

## Overview

`apxm-gui` is a standalone Axum web server that serves a React 19 + TypeScript single-page application. It visualizes AIS workflow graphs using ELK layout, displays compiler optimization diffs, and replays live session traces with streaming updates.

## Architecture

The crate has two layers:

- **Rust backend** (`src/`) -- Axum HTTP server that parses `.air` files via `AirModule`, serves the embedded SPA, and exposes REST APIs for graph data, file browsing, and live session events.
- **TypeScript frontend** (`frontend/`) -- React 19 SPA with Vite, ReactFlow, and ELK for automatic graph layout. Built artifacts are embedded in `src/frontend-dist/`.

## Backend Module Structure

| Module | Description |
|--------|-------------|
| `main` | Axum router, static file serving, CLI argument parsing |
| `api/mod` | REST endpoints for graph parsing, file listing, health |
| `api/live` | SSE endpoints for live session streaming |

## Frontend Structure

| Directory | Description |
|-----------|-------------|
| `api/` | Typed fetch wrappers for backend REST endpoints |
| `components/graph/` | `AisNodeCard`, `GraphCanvas`, `GraphToolbar`, `FileSidebar` |
| `components/shared/` | Reusable UI components |
| `hooks/` | `useElkLayout`, `useKeyboard` custom hooks |
| `layouts/` | `AppShell` navigation layout |
| `store/` | Zustand `appStore` for global state |
| `types/` | TypeScript type definitions (`api.ts`, `graph.ts`, `session.ts`) |
| `views/` | `DashboardView`, `GraphView`, `LiveView`, `ReplayView`, `SessionView` |
| `lib/` | `graphBuilder`, `format`, `constants` utilities |

## Key Exports

- `AppState` -- shared Axum state with file paths and config
- `validate_path` -- path traversal guard for file browser API

## Dependencies

| Crate | Purpose |
|-------|---------|
| apxm-core | Shared types |
| apxm-ais | AIS operation metadata for node rendering |
| apxm-compiler | `AirModule` parsing (via re-export) |

## Running

```bash
apxm gui --file workflow.air --port 18801 --open
```
