# Coder

Coder explores a session workspace with `read`, proposes changes through the
read-only `edit` handler, and prepares a deterministic test command through
the read-only `test` handler. The host must approve `write` and `bash` before
the proposal is applied or the test command runs.

Coder may spawn `explorer`. The child receives only `read` and reports
findings to its parent.
