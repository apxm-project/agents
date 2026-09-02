---
status: accepted
date: 2026-09-02
owner: APXM agents
requires: ADR-0013, ADR-0023
---

# A development inference backend is selected by the environment

## Status and authority

Accepted for the Agents repository. It adds one environment-selected backend
to the Runtime Service composition root. It changes no Port Contract, no
protocol method, no schema and no roster semantics.

## Context

`LocalModelInferencePort::from_backend_roster` builds the Runtime Service's
inference surface from exactly one source: the backend roster at
`$APXM_HOME/config.toml`. Resolution against that surface is exact — an
authored `ModelTargetRef` names the one backend bound to it, and there is no
default, alias or first-available backend.

That is right for a deployment and unusable for development. A Runtime Service
container runs as a system user with no home directory, so its roster is empty;
every `model.call` then fails before send with `model_target_not_registered`,
the invocation commits as `failed`, and `program_invocation_start` answers with
a failure frame. A program that composes one model call with host capabilities
(ADR-0025) cannot be run at all on a laptop or on CI without a provider
account, so the one path that proves the machine end to end cannot be
exercised.

The obvious workarounds are all worse. A default backend would make the exact
resolution inexact for every deployment. A roster file mounted into the image
would put a per-instance concern into a per-deployment artifact and still
depend on `$APXM_HOME` being writable. A hosted provider in CI would make a
deterministic gate depend on a network service and a credential.

## Decision

The Runtime Service admits one **development backend**, selected only by the
environment and never by the roster.

1. **Selection.** `APXM_BACKEND` selects it. The one admitted value is
   `fixture`. Unset, nothing is registered and the roster remains the only
   source of backends. Set to anything else, the selection is refused and the
   refusal is recorded as roster evidence, so an unresolvable target names it.

2. **Exactness is preserved.** `APXM_BACKEND_MODEL` names the exact model
   references the backend serves, as a comma-separated list. It is required:
   a development backend serves the references it was named for and no others,
   so resolution stays exact and the backend is never a fallback for a target
   nobody configured.

3. **A configured provider is never displaced.** Registration binds each named
   reference through the same `bind_model` the roster uses. A reference the
   roster already bound keeps its configured backend, and the conflict is
   recorded rather than resolved.

4. **The answer is a pure function of the request.** The fixture renders the
   request — model reference, prompt, every message with its role,
   temperature, `max_tokens`, `top_p` and every stop sequence — and returns
   `apxm-fixture:<sha256 of that rendering>` with `finish_reason = stop`, no
   tool calls, and a stated four-characters-per-token usage estimate. The same
   request always produces the same completion; a request differing in any
   admitted field produces a different one.

5. **It is unmistakable.** No provider returns a completion whose text is
   `apxm-fixture:` followed by a digest. A development answer is therefore
   distinguishable from a provider answer in a transcript, an observation and
   a run report, by inspection and by a string comparison.

## Operating it

Set both variables on the Runtime Service container or process. The model
reference must be the one the program's `model.call` names:

```sh
APXM_BACKEND=fixture
APXM_BACKEND_MODEL=<the exact model reference the program calls>
```

Several references are one comma-separated value
(`APXM_BACKEND_MODEL=first.model,second.model`). Nothing else is needed: the
backend is in the Runtime Service binary, so there is no endpoint, no base
URL, no credential, no sidecar and no image change. Unsetting `APXM_BACKEND`
returns the service to roster-only resolution with no other edit.

## Consequences

- A program that calls the model runs inside the shipped Runtime Service with
  no provider account, on a laptop and on CI, and its answers are stable across
  runs — so a gate over such a program is deterministic.
- The development path is opt-in per process. A deployment that does not set
  `APXM_BACKEND` is byte-for-byte unaffected: the same roster, the same exact
  resolution, the same failure text when a target is unbound.
- The fixture is not confinement and not a safety boundary. It answers any
  prompt it is asked, so it belongs in development and tests only; the
  environment variable is the whole of the gate, and an operator who sets it in
  a deployment has chosen fixture answers.
- The roster evidence a failed attempt reports now also names development
  selections that did not take effect, so a misspelled selector or an unnamed
  model reference reads as a stated reason instead of a bare unknown model.
