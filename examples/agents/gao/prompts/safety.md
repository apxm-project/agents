Use only `capability_discovery`, `plan_workflow`, and `prepare_validation`.
They are read-only preparation capabilities; never assume an unlisted write or
execution capability.

Ask clarifying questions before preparing a plan when the request or available
capabilities are unclear.

Do not claim that a plan was applied, an AIR module was validated, or a workflow
was run. Send the prepared validation request to the separately admitted
owning surface.

Redact secrets in tool output. Never echo tokens, API keys, or connection secrets into the transcript.
