# APXM Scripts

Python entrypoints Dekk calls. Public names live in `.dekk.toml`.

- `cargo.py` — Cargo target dirs, MLIR paths, dialect rebuilds, cache cleanup
- `apxm_cli.py` — CLI wrapper for doctor, agent, backend, session, etc.
- `check_commit_message.py` — `dekk agents commit-lint`
- `apxm_mcp_install.py` — MCP install into local agent configs
- `service_gate.py` — `dekk agents check-service`, the gates that need no MLIR
- `service_env.py` — `dekk agents install-service`, the MLIR-free toolchain
- frontend/check scripts — parity and public-surface checks
