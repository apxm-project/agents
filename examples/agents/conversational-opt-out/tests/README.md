# Conversational opt-out acceptance variant

This package has the same read/search/write control surface as the bounded
conversational profile, but deliberately omits both `runtime.memory_space` and
`runtime.compaction_policy`. The conversational reference profile runs the same scripted
transcript against both packages and requires this variant to emit no
`context_window_warning` or `context_compacted` events.
