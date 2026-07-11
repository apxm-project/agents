# Gao Python profile

This profile is authored with the APXM Python frontend. It has a local,
read-only `search_papers` capability and an abstract provider-backed web-search
group. A host supplies only an opaque connection reference; credential material
is resolved outside the profile.

The per-turn graph retrieves the local corpus, creates an
`apxm_studio_workflow` draft, and asks the model to answer from those results.
The draft uses the same `text` → `llm` → `output` composition supported by the
Studio canvas.

Validate the profile and emit its artifact:

```bash
PYTHONPATH=crates/compiler/frontend/python \
  python3 examples/python/gao/gao_profile.py
```

Run the deterministic local turn:

```bash
PYTHONPATH=crates/compiler/frontend/python \
  python3 -c 'from examples.python.gao import run_live_turn; print(run_live_turn("How do dependency-driven graphs execute?"))'
```

The profile does not read provider API keys or choose a provider-specific
endpoint. Without a connection reference, the abstract web-search capability
is absent.
