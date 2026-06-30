# Security Policy

## Reporting a vulnerability

If you believe you have found a security-sensitive issue in APXM — for example,
a way for a skill to escape its declared capabilities, an authentication bypass
on `apxm-server`, or any path that reveals secrets stored under `apxm-credentials`
— please report it privately rather than opening a public issue.

Email: **randres2011@gmail.com**

Please include:

- The affected component (e.g. `apxm-server`, `apxm-runtime`, the vLLM fork).
- A reproduction (an AIR graph, a curl invocation, or a Python frontend snippet
  is ideal).
- The impact you believe the issue has.
- Any logs or session directories that demonstrate the behavior.

You should expect an acknowledgement within a few working days. Coordinated
disclosure is preferred: please give the project a reasonable window to
remediate before publishing details.

## Supported versions

APXM is currently in early development (`0.0.x`). Until a `1.0` release, only
the `main` branch and the most recent tagged release receive security fixes.

## Scope

In-scope:

- The Rust workspace under [`crates/`](crates/).
- The Python frontend under [`crates/compiler/frontend/python/`](crates/compiler/frontend/python/).
- The `apxm-server` HTTP and MCP surfaces.
- The `apxm-credentials` store.
- Any sample skill or workflow under [`examples/`](examples/) where the
  vulnerability is in APXM machinery rather than an example-specific bug.

Out of scope (please report upstream):

- The bundled [`external/vllm`](external/vllm) fork (report to the vLLM project,
  or to `apxm-server` only if the bug is in APXM's wrapping of vLLM).
- Third-party agent CLIs invoked over ACP (Claude Code, Codex, etc.).

## Hall of fame

We are happy to publicly credit security researchers who responsibly disclose
issues. Please tell us in your report if you would like to be credited and how
to attribute you.
