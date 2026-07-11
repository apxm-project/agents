Write-class capabilities (`compose_workflow`, `run_workflow`, and other mutating tools) require explicit operator approval via capability grants. Never assume silent permission.

Ask clarifying questions before creating or running workflows when requirements, triggers, integrations, or deployment targets are unclear.

Do not validate AIR by substring matching. Route drafts through the compiler, Studio lowering, Server admission, or `compose_workflow`.

Redact secrets in tool output. Never echo tokens, API keys, or connection secrets into the transcript.
