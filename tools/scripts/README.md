# APXM Scripts

`tools/scripts/` contains Python entrypoints that Dekk calls directly. Keep
public command names in `.dekk.toml`; keep implementation details inside a
script-local package when a command grows beyond one responsibility.

## Command Wrappers

- `cargo.py` owns APXM Cargo invocation details: machine-local target dirs,
  MLIR library path handling, release binary staging, dialect rebuilds, and
  cache cleanup.
- `vllm.py` owns APXM-vLLM operator workflows. Keep the public surface under
  `dekk agents vllm`.
- `release.py` is the Dekk-facing release entrypoint. Key modules live in
  `apxm_release/`:
  - `checks.py` runs release readiness gates.
  - `dist.py` builds archives, Python distributions, and checksums.
  - `publish.py` owns GitHub and PyPI publishing.
  - `cli.py` owns argument parsing.
- `reference_host_receipt.py` builds the canonical `apxm-reference-host`
  executable through the APXM Cargo wrapper and records an exact receipt under
  `.apxm/reference-host/receipts/`.
- `reference_host_lifecycle_receipt.py` consumes one explicit
  `apxm.reference-host-startup-input.v1` artifact plus the exact committed
  release manifest, exercises the canonical JSONL stdin/stdout transport, and
  records a typed lifecycle receipt under `.apxm/reference-host/receipts/`.
- `reference_host_startup_input.py` emits the exact
  `apxm.reference-host-startup-input.v1` artifact under
  `.apxm/reference-host/startup-inputs/` for one explicit clean owner revision
  plus explicit operator-supplied release, port-binding, and resource-ceiling
  digests. It does not sign readiness or mutate committed release manifests.

## Validators and Installers

- `check_commit_message.py` enforces APXM commit-message rules for the
  `dekk agents commit-lint` command.
- `validate_agent_skill.py` validates one instruction-only Agent Skill directory.
- `apxm_mcp_install.py` installs or removes the APXM MCP server in supported
  local agent configs.
- `prompt_as_workflow_smoke.py` runs real-backend MCP prompt-as-workflow
  dogfood scenarios and stores evidence under `.apxm/`.

Do not commit local caches such as `__pycache__/`; they are ignored and can be
removed at any time.
