# Real-World

Complete production workflows demonstrating APXM at scale.

## Why This Matters

These examples combine multiple APXM features (parallelism, multi-agent,
optimization) into end-to-end workflows that solve real problems.

## Examples

- **code_review_council.py** -- Multi-agent code review with parallel reviewers. `dekk apxm execute examples/python/real-world/code_review_council.py`
- **sdlc_pipeline.py** -- Full SDLC: architect designs, coder implements, reviewer verifies. `dekk apxm execute examples/python/real-world/sdlc_pipeline.py`
- **ultrathink_coder.py** -- Extended reasoning for complex coding tasks. `dekk apxm execute examples/python/real-world/ultrathink_coder.py`
- **codex_claude_fix.py** -- Claude diagnoses, Codex fixes, Claude verifies. `dekk apxm execute examples/python/real-world/codex_claude_fix.py`
- **autofix_loop.py** -- Automated validate/fix/verify pipeline. `dekk apxm execute examples/python/real-world/autofix_loop.py`

## Key API

```python
# Multi-agent SDLC
architect = g.spawn("architect", profile=claude, cwd=cwd)
coder = g.spawn("coder", profile=codex, cwd=cwd)

architect.ask("Design the feature")
design = architect.get_last_node()

coder_result = g.communicate(target_agent="coder", message="{design}")
```

## Learn More

- [self-hosted/](../self-hosted/) -- APXM building APXM
- [patterns/](../patterns/) -- Building blocks used in these workflows
