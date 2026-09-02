---
status: accepted
date: 2026-09-02
owner: APXM agents
requires: ADR-0023
---

# Submitted-source capture runs inside a kernel boundary

## Status and authority

Accepted for the Agents repository. Subordinate to ADR-0023, which made the
Compilation Service the only thing that turns submitted source into an artifact
and made Python `apxm_program` and TypeScript `@apxm/frontend` the complete
authoring set. This record does not reopen that decision, the capture
translation, or the closed source-diagnostic vocabulary. It states what the
kernel confines a capture child to, and the one operator flag that answers a
kernel which cannot provide it.

## Context

Capturing typed intent from submitted source runs an authoring frontend, and
both frontends resolve an authored callback through their language's module
system, so the submitted text is evaluated. The evaluation has been confined
from inside the interpreter: the Python harness runs an isolated interpreter
behind an audit hook that rejects process, network, dynamic-code and filesystem
events, with a zero file-size limit, a CPU limit, an address-space limit and a
zero-process limit; the TypeScript harness runs under Node's permission model,
which grants read of one package root and no write at all.

That is one wall, and it is the wall a submitted program is best placed to
attack. It is written in the language the attacker controls, it depends on the
interpreter emitting an audit event for every reachable operation, and on the
TypeScript side it bounds neither CPU time nor memory. A hosted deployment that
accepts third-party source needs the operating system to answer as well, so that
a hole in an interpreter reaches a second closed door rather than the host.

## Decision

**Every capture child runs inside a kernel boundary, applied by the source port
before the interpreter execs.** On Linux the boundary is three things:

1. **A Landlock ruleset.** The child may read and execute the authoring frontend
   package root, the declared interpreter driver and the driver's own runtime
   prefix, and the shared-library roots the interpreter links against; it may
   read the architecture-independent runtime data beside them, `/proc`, the
   loader cache, the local time zone, and the random and null devices. It may
   read, write and create inside exactly one per-capture scratch directory,
   which the port creates, names to the child as `TMPDIR`, and removes when the
   capture ends. Everything else is denied by the kernel: `/etc/passwd`, the
   home directories, the artifact directory, the rest of the checkout.
   Executable rights stop at the library roots, so no system command is
   executable inside the boundary and a submitted program cannot exec a shell
   even where it can name one.

2. **A seccomp filter.** Every socket domain but `AF_UNIX` is refused with
   `EPERM`, so the network is closed whichever interpreter is running and
   whether or not that interpreter emits an audit event. A new process is
   refused however the child asks for one — `fork`, `vfork`, and any `clone`
   that is not creating a thread — while thread creation, which both
   interpreters need, is allowed; that holds whatever user the service runs as,
   which a process ceiling alone does not. Tracing, namespace, mount, module
   and key-management calls are refused outright. `no_new_privs` is set, so
   nothing the child execs can regain privilege.

3. **Resource ceilings.** CPU time, writable data, produced file size and core
   dumps are bounded, and the new-process ceiling is zero, so every fork fails.
   The CPU ceiling is below the port's own wall-clock capture timeout, so a
   program that only spins is ended by the kernel rather than by the supervising
   thread. The TypeScript bridge additionally carries Node's own old-space
   ceiling on its command line, because Node has no interpreter resource limits
   of its own and that ceiling therefore holds on every host.

The interpreter-level confinement stays exactly as it is. It is the inner wall,
it is tighter than the kernel boundary in the places it can be, and neither wall
is relaxed because the other exists.

### The operator flag

`APXM_CAPTURE_CONFINEMENT` selects how a kernel that cannot provide the boundary
is answered.

| Value | Meaning |
| --- | --- |
| unset, or anything but `permissive` | `enforce`. A Linux host without Landlock or without seccomp filtering refuses every capture with `frontend_unavailable`. |
| `permissive` | Development only. Apply whatever the kernel offers and capture anyway. |

A typo is `enforce`, so no misspelling opens the boundary. The Compilation
Service prints its capture-confinement readiness to standard error before it
serves its first request — the boundary identifier, the host, the mode, the
status, the Landlock ABI the kernel reported, and one sentence an operator can
act on — so a deployment onto a kernel without Landlock is visible at start
rather than at the first refused compile.

### A host that is not Linux

macOS provides no Landlock ruleset and no seccomp filter. There the boundary is
a **documented no-op**: capture runs behind the interpreter-level confinement
alone, and readiness reports `unsupported_platform` with that sentence in it. A
development laptop is not a deployment target, and there is nothing for the
`enforce` mode to fail closed against, so `enforce` does not refuse capture
there. The conformance for the boundary is Linux conformance and runs on Linux.

## Consequences

* A hostile program that reaches for `/etc/passwd`, opens an internet socket,
  execs a system binary, forks a shell, allocates without bound, or spins is
  refused by the kernel and the capture ends in one closed source diagnostic —
  `source_rejected` where the interpreter reports the refusal, and
  `frontend_unavailable` where the kernel ended the child before it could
  report anything. Neither produces a graph, and neither produces an artifact.
* The boundary is built in the parent and entered by the child before exec, so
  the pre-exec step allocates nothing and makes only the calls that restrict it.
* A capture may write exactly one directory. A frontend that ever needs a
  temporary file has one, named the way an interpreter looks for one, and it
  disappears with the capture.
* The service image must run on a kernel with Landlock and seccomp. That is the
  common case for a hosted deployment and the reason the flag exists for the
  case it is not.
* Shipping this boundary changes the service binary, so it reaches a consumer
  only in the next cohort, with the release manifest and image labels re-cut
  from these bytes.
