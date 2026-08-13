# apxm-backends

This crate contains optional inference and capability implementations. It is
an adapter collection, not a second semantic runtime or provider-selection
authority.

The canonical runtime receives one exact admitted implementation through a
typed Port binding. It never searches a registry, picks the first healthy
provider, substitutes a model, or falls back after failure.

Each adapter must preserve request identity, typed context, cancellation,
retry/reconciliation, outcome-unknown semantics, usage, and evidence. The
abstract-machine source and AIR remain provider-neutral.
