---
name: extend
description: Run the architect-implement-review workflow to add a feature to APXM
user-invocable: true
---

# Extend

Run the APXM multi-agent development workflow as a graph execution.
APXM spawns Claude (architect) and Codex (coder) via ACP, orchestrates
design -> implement -> review automatically, and writes all outputs to a
session folder.

## Steps

1. Execute the graph, passing the feature request as an argument:
   ```bash
   dekk apxm execute .agents/skills/extend/extend.air --emit-session "$ARGUMENTS"
   ```
   
   **Note:** To ensure sessions land in `~/.apxm/sessions/`, use `--emit-session` without
   a label (auto-generates path), or pass an absolute path:
   ```bash
   dekk apxm execute .agents/skills/extend/extend.air --emit-session $HOME/.apxm/sessions/my-feature "$ARGUMENTS"
   ```

2. Monitor progress in the session folder:
   - `ls ~/.apxm/sessions/` -- find the latest session
   - `cat ~/.apxm/sessions/<id>/live.json` -- live progress (completed/total)
   - `cat ~/.apxm/sessions/<id>/node_statuses.json` -- per-node status
   - `cat ~/.apxm/sessions/<id>/results.json` -- all node outputs

3. Replay the session timeline:
   ```bash
   dekk apxm replay ~/.apxm/sessions/<id>
   ```

4. After completion, inspect the architect's review verdict in the last
   node output. If "iterate", the session folder contains all context
   needed to understand what to fix.
