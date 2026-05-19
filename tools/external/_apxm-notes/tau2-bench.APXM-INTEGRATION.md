# APXM ↔ τ²-bench integration notes

Vendored at `tools/external/tau2-bench/` on 2026-05-19. Pinned SHA in
`external-pins.txt`. Refresh deliberately (Plan 05 risk #2).

## Wire-up summary (Plan 05 EVAL)

τ²-bench routes all model traffic through [LiteLLM](https://github.com/BerriAI/litellm).
The agent + user model arguments accept any LiteLLM-supported provider
string. For a custom OpenAI-compatible endpoint (vLLM, HAL adapter,
etc.) the path is the `openai/`-prefix syntax with two env vars:

```bash
export OPENAI_API_BASE="http://127.0.0.1:8916/v1"   # APXM-vLLM zoo
export OPENAI_API_KEY="dummy"                         # required by litellm
tau2 run \
  --domain airline \
  --agent-llm openai/gpt-oss-120b \
  --user-llm  openai/gpt-oss-120b \
  --num-trials 1 --num-tasks 1
```

For the **apxm-on arm**, route through `tools/hal_adapter/server.py`
(already shipped) with `--backend apxm-on` and point `OPENAI_API_BASE`
at the adapter's port. The flat-http arm points directly at the zoo.

## Install path

Python ≥ 3.12 required (we have 3.12.3). The vendored project's
`pyproject.toml` declares dependencies (rich, fastapi, uvicorn, pandas,
loguru, litellm, tenacity, deepdiff, addict, pyyaml, typer, requests,
numpy, httpx). Install into a tau2-scoped venv to avoid polluting the
main apxm `.venv`:

```bash
python3 -m venv .venv-tau2
.venv-tau2/bin/pip install -e tools/external/tau2-bench
```

## Domains available

`airline`, `retail`, `telecom`, `mock`, `banking_knowledge`. Task
counts: airline ~50, retail ~115, telecom ~480 (`tasks_small.json` has
a smaller cell-shaped subset). For first-pass Plan 05 EVAL, **airline**
is the smallest end-to-end domain and the canonical paper cell.

## What this does NOT include

- Voice extras (`uv sync --extra voice`) — not needed for text agent
  evaluation.
- Knowledge extras — not needed for the bare τ²-bench loop.
- Authentication for any third-party service — APXM-vLLM serves the
  agent model locally; tau2's user simulator also runs against the
  same local endpoint.

## Next step (deferred — separate session)

Create `.venv-tau2`, install, run a `--num-tasks 1` smoke on `airline`
through the HAL adapter (apxm-on arm) and direct (flat-http arm).
Capture the two pass-rate counters into the Plan 05 EVAL CSV per
Plan 00 §2 task-quality column set.
