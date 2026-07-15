# Conversational opt-out acceptance variant

This package has the same read/search/write control surface as the bounded
conversational profile, but deliberately omits both `runtime.memory_space` and
`runtime.session_prefix`. The conversational reference profile runs the same scripted
transcript against both packages and proves that program-owned context behavior
does not depend on undeclared manifest runtime controls.
