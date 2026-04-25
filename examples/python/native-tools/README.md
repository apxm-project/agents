# Native Tools

Native Python agent and tool examples.

## Examples

- **handoff_demo.py** -- Two native agents with an explicit handoff.

## Usage

```bash
python3 examples/python/native-tools/handoff_demo.py
```

This is an in-process mock-backed example. It uses `apxm.run(..., mock=True)`,
so it does not require a real backend.
