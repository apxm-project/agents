---
name: add-provider
description: Add a new LLM provider to APXM
user-invocable: false
---

# Add Provider

How to add a new LLM provider to APXM. Providers are the backend implementations that handle LLM API calls.

## Procedure

### 1. Choose a protocol

If the new provider uses an existing wire protocol (e.g., OpenAI-compatible), use an existing `ProviderProtocol` variant. Otherwise, add a new variant.

**Adding a new protocol variant** (rare — most providers are OpenAI-compatible):

Edit `crates/core/apxm-core/src/types/provider_spec.rs`:

```rust
pub enum ProviderProtocol {
    // ...existing variants...
    MyProtocol,
}
```

Update `as_str()`, `from_str()`, and `all_variants()` in the same file.

### 2. Add to BUILTIN_PROVIDERS

In `crates/core/apxm-core/src/types/provider_spec.rs`, add an entry:

```rust
pub const BUILTIN_PROVIDERS: &[BuiltinProviderSpec] = &[
    // ...existing entries...
    BuiltinProviderSpec {
        id: "my-provider",
        api_key_env_var: Some("MY_PROVIDER_API_KEY"),
        default_base_url: Some("https://api.my-provider.com/v1"),
        requires_api_key: true,
        protocol: ProviderProtocol::OpenAI,  // or new protocol
        aliases: &["my-alias"],
    },
];
```

### 3. Implement backend (if new protocol)

Create `crates/runtime/apxm-backends/src/llm/backends/my_protocol.rs`:

- Implement the `LLMBackend` trait
- Handle request/response mapping
- Add match arm in the backend factory

### 4. Regenerate codegen

```bash
dekk apxm codegen frontend      # Updates providers.py
dekk apxm codegen typescript    # Updates generated.ts
```

### 5. Register and test

```bash
dekk apxm backend add my-key --type cloud --protocol my-protocol --api-key <API_KEY>
dekk apxm backend test my-key
```

## Key Files

| File | Role |
|------|------|
| `crates/core/apxm-core/src/types/provider_spec.rs` | ProviderProtocol enum + BUILTIN_PROVIDERS |
| `crates/runtime/apxm-backends/src/llm/backends/` | Backend implementations |
| `crates/compiler/apxm-frontend/python/apxm/_generated/providers.py` | Generated Python provider specs |
| `crates/tools/apxm-gui/frontend/src/types/generated.ts` | Generated TypeScript provider types |
