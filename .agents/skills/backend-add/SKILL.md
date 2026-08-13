---
name: backend-add
group: Runtime
description: Register a model-inference implementation or exact target binding.
user-invocable: true
---

Load `_shared/apxm-development-rules.md` before broad work.

Backend implementations are adapters behind the exact ModelInferencePort. A
backend registration names an implementation and its contract; it must not
become provider discovery, fallback routing, or a second execution semantics.

Before implementation, define the exact target identity, Port Contract,
binding digest, credentials reference, typed request/outcome schema, and
unavailable/outcome-unknown behavior. Configuration must fail closed when a
required field is absent.

Verify with the focused inference and execution tests through Dekk. Keep
provider-specific deployment and fleet operations outside this repository.
