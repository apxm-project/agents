---
name: apxm-backend-add
group: Domain
description: Use when registering a new APXM inference backend (cloud, on-prem, or local). Enforces hard-fail-at-config-time resolver behavior.
user-invocable: true
---

# APXM Backend Add

Load `_shared/apxm-development-rules.md` before broad work.

## Authority commands

```bash
dekk apxm backend list                  # what's registered
dekk apxm backend add                   # register a new backend
dekk apxm backend test <name>           # connectivity probe
dekk apxm backend remove <name>
dekk apxm backend start <name>          # local backends only
dekk apxm backend stop <name>
dekk apxm backend status <name>
dekk apxm backend add-model <name> <model-id>
```

## Rules

- **No fallback chains.** No `cfg or env or "default"`, no
  `apxm_endpoints_available`-style flags, no resolver "last resort"
  branches, no silent round-robin fallback.
- **Hard-fail at config time.** If a required field is missing, error
  loudly at load — never paper over with a default.
- **`model.id` must match the backend's `served_model_name` exactly.**
  Bare name (e.g. `gpt-oss-120b`), not HF repo (`openai/gpt-oss-120b`).
  Mismatch → silent 404 storm. See
  `feedback_apxm_model_id_must_match_served`.
- **Don't shadow `~/.apxm/config.toml`** with a project-local file
  that only sets `data-dir`. The resolver is first-wins, not merge —
  see `apxm_config_resolver_does_not_merge`. Either keep all config in
  one file or fully merge the backend block.
- For vLLM backends, prefer `dekk apxm vllm zoo-apply` over `backend
  add` — the zoo manifest is the operator surface.

## Workflow

```bash
# Cloud / on-prem (manual entry):
dekk apxm backend add
# Follow the prompts; supply api_base, served_model_name, credentials
# (from env, never literal).
dekk apxm backend test <name>
dekk apxm backend add-model <name> <model.id>

# vLLM:
# Use the zoo path instead — see apxm-model-zoo-operate.
```

## Diagnostics

- `dekk apxm backend test <name>` — connectivity + auth.
- `dekk apxm doctor` — env / resolver sanity.
- `dekk apxm vllm probe <api_base>` — APXM-vLLM-specific route probe.

## Touch points (implementing a new provider)

Implementing a provider (vs. registering one) spans:
`crates/runtime/apxm-backends/src/<provider>/{mod.rs,client.rs,config.rs}`,
`…/registry.rs` (enum + dispatch), `…/apxm-runtime/src/executor.rs` (if a
dispatch hint is needed), `config/backends.example.toml`,
`tests/backends/<provider>_smoke.rs`. vLLM dispatch hints go through
`apxm-fork-vllm-rebase` (cherry-pick against `apxm-rebase-v0.21.0`; never edit
`external/vllm` in place).

## Anti-patterns

- Adding a backend without `backend test` afterward — the registry can
  hold a broken config silently.
- Hardcoding `api_base = "http://127.0.0.1:8916"` — port `8916` lints
  as `hardcoded-port-8916` outside the allocator-range default.
- Adding a credential as a literal in config rather than `env:<NAME>`.
- Creating a project-local `.apxm/config.toml` that only sets
  `data-dir` (shadows user-global backends).
- A literal contract string (e.g. `"reuse_group"`) in a handler — promote it
  to a `graph_attrs::*` / `metrics_keys::*` constant
  (`feedback_attribute_dual_naming`).
