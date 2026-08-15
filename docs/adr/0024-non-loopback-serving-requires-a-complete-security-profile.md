---
status: accepted
date: 2026-08-15
owner: APXM agents
requires: ADR-0023
---

# Non-loopback Event and OpenAI HTTP serving requires a complete security profile

## Status and authority

Accepted for the Agents repository. Subordinate to ADR-0023. Loopback Event HTTP and OpenAI protocol serving may ship with request, stream, concurrency, deadline, and payload limits. Binding any non-loopback address is impossible unless this profile is fully configured and its conformance suite passes.

## Decision

A non-loopback listener requires all of: authentication, authorization, TLS, tenant/resource boundaries, audit, admission-issuer trust, replay/idempotency, secret handling, and abuse limits. Incomplete configuration fails closed. `apxm-event-http::BindPolicy::NonLoopback { security_profile_complete: false }` is not allowed.

## Consequences

Default bind is loopback. Operators cannot discover a bind-address flag that weakens this record.
