# Real-World

Complete production workflows demonstrating APXM at scale.

## Why This Matters

These examples combine multiple APXM features (parallelism, multi-agent,
optimization) into end-to-end workflows that solve real problems.

## Requirements

Most workflows in this directory spawn generated ACP profiles such as
`claude` and `codex`. Verify the profiles with `dekk apxm agent list` and
test the local CLI/auth setup with `dekk apxm agent test <name>` before
executing them. The checked-in profiles run through Node/npm wrappers.

## Examples

- **code_review_council.py** -- Multi-agent code review with parallel reviewers. `dekk apxm execute examples/python/real-world/code_review_council.py`
- **sdlc_pipeline.py** -- Full SDLC: architect designs, coder implements, reviewer verifies. `dekk apxm execute examples/python/real-world/sdlc_pipeline.py`
- **ultrathink_coder.py** -- Extended reasoning for complex coding tasks. `dekk apxm execute examples/python/real-world/ultrathink_coder.py`
- **codex_claude_fix.py** -- Claude diagnoses, Codex fixes, Claude verifies. `dekk apxm execute examples/python/real-world/codex_claude_fix.py`
- **autofix_loop.py** -- Automated validate/fix/verify pipeline. `dekk apxm execute examples/python/real-world/autofix_loop.py`

## Key API

```python
from apxm._generated.agents import claude, codex

# Multi-agent SDLC
architect = g.spawn("architect", profile=claude, cwd=cwd)
coder = g.spawn("coder", profile=codex, cwd=cwd)

design = architect.ask("Design the feature")

coder_result = coder.ask("{design}")
```

## Learn More

- [self-hosted/](../self-hosted/) -- APXM building APXM
- [patterns/](../patterns/) -- Building blocks used in these workflows
