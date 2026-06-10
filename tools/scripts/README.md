# APXM Scripts

`tools/scripts/` contains Python entrypoints that Dekk calls directly. Keep
public command names in `.dekk.toml`; keep implementation detail inside a
script-local package when a command grows beyond one responsibility.

## Command Wrappers

- `cargo.py` owns APXM Cargo invocation details: machine-local target dirs,
  MLIR library path handling, release binary mirroring, dialect rebuilds, and
  cache cleanup.
- `vllm.py` owns APXM-vLLM operator workflows. Keep the public surface under
  `dekk apxm vllm`; do not add compatibility commands for removed service
  flows.
- `release.py` is the Dekk-facing release entrypoint. The implementation lives
  in `apxm_release/`:
  - `checks.py` runs release readiness gates.
  - `dist.py` builds archives, Python distributions, and checksums.
  - `publish.py` owns GitHub and PyPI publishing.
  - `cli.py` owns argument parsing.

## Validators And Installers

- `check_commit_message.py` enforces APXM commit-message rules for hooks and
  Dekk.
- `check_no_legacy_vllm.py` enforces the no-legacy and no-silent-fallback vLLM
  policy.
- `validate_skill.py` validates a single skill bundle.
- `validate_pack.py` validates builtin skill-pack catalogs.
- `apxm_mcp_install.py` installs or removes the APXM MCP server in supported
  local agent configs.
- `install_git_hooks.py` installs APXM-managed git hooks.
- `prompt_as_workflow_smoke.py` runs real-backend MCP prompt-as-workflow
  dogfood scenarios and stores evidence under `.apxm/`.

Do not commit local caches such as `__pycache__/`; they are ignored and can be
removed at any time.
