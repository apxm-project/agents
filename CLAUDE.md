# apxm

Language: Rust

## Commands

- **build**: `dekk apxm build` -- Build the APXM compiler and runtime from source
- **test**: `dekk apxm test` -- Run the test suite (1000+ tests, no MLIR required)
- **compile**: `dekk apxm compile <graph.apxm>` -- Compile graph to optimized .apxmobj artifact
- **execute**: `dekk apxm execute <graph.apxm>` -- Compile and run a graph in one step
- **run**: `dekk apxm run <artifact.apxmobj>` -- Execute a pre-compiled artifact
- **decompile**: `dekk apxm decompile <artifact.apxmobj>` -- Reverse-map artifact back to graph JSON
- **validate**: `dekk apxm validate <graph.apxm>` -- Validate graph against AIS contract
- **analyze**: `dekk apxm analyze <graph.apxm>` -- Parallelism analysis, critical path, speedup estimate
- **explain**: `dekk apxm explain <graph.apxm>` -- Human-readable walkthrough of a graph
- **view**: `dekk apxm view <graph.apxm>` -- Open interactive graph visualizer in browser
- **ops**: `dekk apxm ops list` / `dekk apxm ops show <OP>` -- Browse AIS operations
- **template**: `dekk apxm template list` / `dekk apxm template show <name>` -- Starter graph patterns
- **task/merge**: `dekk apxm task merge <a.apxm> <b.apxm> --name <name>` -- Merge graph fragments
- **init**: `dekk apxm init <name>` -- Scaffold a new project
- **doctor**: `dekk apxm doctor` -- Diagnose environment health
- **backend/add**: `dekk apxm backend add <name> --type <type> --protocol <proto>` -- Register inference backend
- **debug**: `dekk apxm execute <graph> --trace <level>` -- Debug workflow execution
- **worktree**: `dekk worktree create <branch>` -- Manage isolated git worktrees

## Build

```bash
dekk apxm build
```

## Test

```bash
dekk apxm test
```
