# ACP and script-worker sandbox seam

APXM routes one-shot commands and long-running subprocesses through the same
backend registry without weakening a request when a backend is unavailable.

## Runtime contract

- `SandboxRegistry::select_for_request` considers availability, minimum
  isolation, network requirements, and each backend's validation result.
- One-shot capability and `EXC` commands use `create_session`, `execute`, and
  `destroy_session`.
- Long-running ACP agents, reverse terminals, Python workers, and TypeScript
  workers use `SandboxBackend::wrap_command`.
- `wrap_command` is fallible and returns a typed `WrappedCommand` containing the
  rewritten launch specification and backend-owned lifecycle guard.
  `WrappedCommand::spawn` consumes that specification and returns a child handle
  that retains the guard until the process handle is dropped.
- Callers provide the complete child environment. APXM builds it from a small
  host allowlist plus explicit runtime overrides, validates every entry, clears
  inherited state, and places `--` before assignments passed through `env`.

ACP sandboxing remains profile-controlled. A profile that requests sandboxing
fails closed when no backend can satisfy its networked, long-running process
requirements. Reverse terminals use the same selected backend so an isolated
agent cannot escape through `terminal/create`.

## Linux backends

### Bubblewrap

Bubblewrap is registered only when an isolation command succeeds; executable
presence or `bwrap --version` is not sufficient. It provides:

- read-only host root with explicit writable mounts, without a readable-path
  allowlist;
- ephemeral `/tmp`;
- a masked `/run/user` tree so host user-bus sockets are unreachable;
- user and PID namespaces;
- a separate network namespace for no-network requests.

The wrapper preserves live stdio for ACP and worker protocols. Networked ACP
processes retain their network namespace while the user runtime socket tree
remains masked.

### Systemd user service

The systemd backend is registered only when a harmless restricted command
succeeds, the same local socket operation succeeds without filtering, and the
filtered operation fails with `EPERM`. It provides:

- `NoNewPrivileges`, SUID/SGID restriction, locked personality, IPC cleanup,
  private umask, native syscall architecture, and a syscall denylist;
- `@network-io` syscall denial for no-network requests, including Unix sockets;
- unique transient units with control-group kill semantics;
- an owned child handle that stops the unit when the process handle is dropped;
- explicit unit cleanup on one-shot completion and timeout.

The systemd backend does **not** claim per-path filesystem restriction on hosts
where user-service mount controls are ineffective. Selection reports this as a
bounded degraded guarantee. Script handlers therefore have no direct network
access and must use admitted capabilities for external I/O.

## Environment boundary

Child processes receive only `PATH`, `HOME`, locale/terminal values, loader
paths, virtual-environment state, and explicit runtime overrides. The outer
systemd launcher receives only the user D-Bus address and `XDG_RUNTIME_DIR`;
those values are not inherited by sandboxed children unless explicitly admitted
as runtime overrides.

Environment names that are empty, option-like, contain `=`, or contain NUL are
rejected. Values containing NUL are also rejected.

## Verification

Focused tests cover:

- wrapper argument and environment construction;
- unavailable and non-enforcing backend probes;
- network denial with the matching unrestricted control probe;
- nested `systemd-run --user` escape rejection;
- child-owned transient-unit cleanup behavior;
- ACP session and terminal environment propagation;
- systemd and bubblewrap capability reporting without readable-path claims.

Run the runtime gates through `dekk agents test`, `dekk agents check`, and
`dekk agents doctor`.
