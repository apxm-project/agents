# Model Profiles Design

**Status**: Design
**Date**: 2026-04-06
**Author**: APXM Team

## Overview

Model profiles provide a declarative way to specify model requirements in APXM graphs without hardcoding specific model names. This document outlines two implementation paths: a conservative allowlist-based governance approach (Plan A) and a full profile-based capability abstraction system (Plan B).

## Problem Statement

Current APXM graphs hardcode model names in node attributes:
```json
{"id": 1, "op": "ASK", "model": "claude-opus-4-6", ...}
```

This creates several problems:
1. **Portability**: Graphs break when specific models are unavailable or deprecated
2. **Governance**: No enforcement of approved model lists for regulated environments
3. **Maintenance**: Changing models requires editing multiple graph files
4. **Semantics**: Intent ("I need reasoning capability") is lost in implementation details

## Core Reframing: Three Separate Concerns

These concerns must NOT be conflated:

| Concern | Question | Domain | When |
|---------|----------|--------|------|
| **Capability Abstraction** | "I need a reasoning-tier model" | Semantic declaration | Compile-time (optional) |
| **Governance Policy** | "Only use approved models" | Security/compliance | Compile-time (enforcement) |
| **Health Routing** | "Don't use degraded models" | Infrastructure resilience | Runtime (ALREADY EXISTS) |

**Key insight**: APXM's `ModelRouter` already handles concern #3 via circuit breakers. We only need to address #1 and #2.

## Key Principles

### P1: Runtime vs Compile-Time Separation
**Rule**: If it changes faster than code, it's runtime config not compile-time data.

- Model health state changes minute-to-minute → runtime circuit breakers (done)
- Model approval lists change monthly → compile-time validation (Plan A)
- Model capabilities change yearly → compile-time abstractions (Plan B)

### P2: Artifact Portability
**Never** embed routing decisions or health data in compiled artifacts.

Artifacts must be portable across environments. Runtime config (backends, health state, profiles) lives in `~/.apxm/config.toml` and session state, never in `.apxmobj` files.

### P3: No Compile-Time Health Checks
**Never** query external APIs during compilation.

Network failures, stale caches, and 8-second build penalties break the developer experience. Health checks belong in the runtime `ModelRouter`.

### P4: Leverage Existing Systems
`ModelRegistry` + tags + circuit breakers already handle health routing. Don't duplicate.

## Plan A: Conservative (Ship in 1-2 days)

### Scope
Allowlist enforcement at compile time. No new abstractions.

### Config Schema
Add to `~/.apxm/config.toml`:
```toml
[models]
allowlist = [
  "claude-sonnet-4-5@20250929",
  "claude-opus-4-6",
  "gpt-4o-mini",
  "gemini-2.0-flash"
]
```

### Compiler Pass
**File**: `crates/apxm-compiler/src/passes/validate_model_allowlist.rs`

```rust
pub fn validate_model_allowlist(graph: &Graph, config: &Config) -> Result<()> {
    let Some(allowlist) = &config.models.allowlist else {
        return Ok(()); // No allowlist configured, skip validation
    };

    for node in &graph.nodes {
        if let Some(model) = node.attributes.get("model") {
            if !allowlist.contains(&model.to_string()) {
                return Err(Error::validation(
                    format!("Model '{}' not in allowlist for node '{}'", model, node.name)
                ));
            }
        }
    }
    Ok(())
}
```

### Implementation
**Files to create**:
- `crates/apxm-compiler/src/passes/validate_model_allowlist.rs`

**Files to modify**:
- `crates/apxm-core/src/config.rs` — add `models: Option<ModelsConfig>` with `allowlist: Option<Vec<String>>`
- `crates/apxm-compiler/src/passes/mod.rs` — wire the pass after type checking

### Behavior
- If `models.allowlist` is configured: validate all `model` attributes against it
- If model not in allowlist: **hard fail** with validation error
- If no allowlist configured: pass silently (backward compatible)

### Example Error
```
Error: Model 'claude-opus-5' not in allowlist for node 'reasoning_step'
Allowed models: claude-sonnet-4-5@20250929, claude-opus-4-6, gpt-4o-mini
```

## Plan B: Bold (Ship in 1 week, includes Plan A)

### Scope
Full profile-based capability abstraction + allowlist enforcement.

### Core Concepts

#### ModelProfile
Declarative capability requirement with prioritized candidate list.

```rust
// crates/apxm-core/src/model_profiles.rs
pub struct ModelProfile {
    pub name: String,                           // e.g., "reasoning-tier"
    pub description: String,                    // Human-readable purpose
    pub tags: Vec<String>,                      // Searchable tags
    pub min_context_window: Option<usize>,      // Optional constraints
    pub max_cost_per_1k_input: Option<f64>,     // Optional cost ceiling
    pub candidates: Vec<ProfileCandidate>,      // Ordered by priority
}

pub struct ProfileCandidate {
    pub model: String,      // Model name (must exist in ModelRegistry)
    pub priority: u32,      // 1 = highest priority
}
```

#### ProfileRegistry (Runtime)
Loads profiles from `~/.apxm/model_profiles.toml`.

```rust
// crates/apxm-runtime/src/model_router/profile_registry.rs
pub struct ProfileRegistry {
    profiles: HashMap<String, ModelProfile>,
}

impl ProfileRegistry {
    pub fn from_config(path: &Path) -> Result<Self>;
    pub fn get_profile(&self, name: &str) -> Option<&ModelProfile>;
    pub fn validate_candidate(&self, model: &str, registry: &ModelRegistry) -> bool;
}
```

#### ProfileRouter (Runtime)
Selects healthy model from profile candidates using circuit breakers.

```rust
// crates/apxm-runtime/src/model_router/profile_router.rs
pub struct ProfileRouter<'a> {
    profile_registry: &'a ProfileRegistry,
    model_router: &'a ModelRouter,
}

impl ProfileRouter<'_> {
    pub fn select_from_profile(&self, profile_name: &str) -> Result<String> {
        let profile = self.profile_registry.get_profile(profile_name)?;

        // Iterate candidates by priority (sorted 1, 2, 3, ...)
        for candidate in profile.candidates.iter().sorted_by_key(|c| c.priority) {
            if self.model_router.is_model_healthy(&candidate.model) {
                return Ok(candidate.model.clone());
            }
        }

        Err(Error::no_healthy_candidates(profile_name))
    }
}
```

### Config Schema

**`~/.apxm/model_profiles.toml`**:
```toml
[[profile]]
name = "reasoning-tier"
description = "High-capability reasoning models for complex analysis"
tags = ["reasoning", "analysis", "research"]
min_context_window = 128000

  [[profile.candidate]]
  model = "claude-opus-4-6"
  priority = 1

  [[profile.candidate]]
  model = "gpt-4o"
  priority = 2

  [[profile.candidate]]
  model = "gemini-2.0-pro"
  priority = 3

[[profile]]
name = "fast-draft"
description = "Fast, cost-effective models for drafting and iteration"
tags = ["draft", "fast", "cheap"]
max_cost_per_1k_input = 0.0003

  [[profile.candidate]]
  model = "claude-sonnet-4-5@20250929"
  priority = 1

  [[profile.candidate]]
  model = "gpt-4o-mini"
  priority = 2
```

**Allowlist in `~/.apxm/config.toml`** (optional):
```toml
[models]
allowlist = [
  "claude-sonnet-4-5@20250929",
  "claude-opus-4-6",
  "gpt-4o-mini",
  "gemini-2.0-flash",
  "gemini-2.0-pro"
]
```

### Graph Schema

Nodes use `model_profile` instead of `model`:
```json
{
  "name": "research-workflow",
  "nodes": [
    {
      "id": 1,
      "op": "ASK",
      "name": "deep_analysis",
      "model_profile": "reasoning-tier",
      "attributes": {...}
    },
    {
      "id": 2,
      "op": "ASK",
      "name": "draft_summary",
      "model_profile": "fast-draft",
      "attributes": {...}
    }
  ]
}
```

**Backward compatibility**: Nodes with `model` attribute continue to work (direct model specification).

### Compiler Pass
**File**: `crates/apxm-compiler/src/passes/validate_model_profile.rs`

Validates:
1. Profile exists in registry
2. All candidates exist in ModelRegistry
3. All candidates pass allowlist (if configured)

```rust
pub fn validate_model_profile(
    graph: &Graph,
    profile_registry: &ProfileRegistry,
    model_registry: &ModelRegistry,
    config: &Config
) -> Result<()> {
    for node in &graph.nodes {
        if let Some(profile_name) = node.attributes.get("model_profile") {
            let profile = profile_registry.get_profile(profile_name)
                .ok_or_else(|| Error::unknown_profile(profile_name))?;

            for candidate in &profile.candidates {
                // Check candidate exists in ModelRegistry
                if !model_registry.has_model(&candidate.model) {
                    return Err(Error::unknown_model(&candidate.model, profile_name));
                }

                // Check candidate passes allowlist (if configured)
                if let Some(allowlist) = &config.models.allowlist {
                    if !allowlist.contains(&candidate.model) {
                        return Err(Error::candidate_not_in_allowlist(
                            &candidate.model, profile_name
                        ));
                    }
                }
            }
        }
    }
    Ok(())
}
```

### Runtime Flow

```
User request arrives at ASK node with model_profile="reasoning-tier"
  ↓
ProfileRouter.select_from_profile("reasoning-tier")
  ↓
Load ModelProfile from ProfileRegistry
  ↓
Iterate candidates sorted by priority [1→2→3]
  ├─ Check: ModelRouter.is_model_healthy("claude-opus-4-6")
  │   └─ Circuit breaker: CLOSED → ✅ healthy
  └─ Return: "claude-opus-4-6"
  ↓
ModelRouter.route_model("claude-opus-4-6") → BackendConfig
  ↓
Execute request on backend
```

**If first candidate unhealthy**:
```
ProfileRouter tries candidate priority=1 → OPEN circuit → skip
  ↓
ProfileRouter tries candidate priority=2 → CLOSED circuit → ✅ use this
  ↓
Return: "gpt-4o"
```

### Implementation

**Files to create**:
- `crates/apxm-core/src/model_profiles.rs` — core types (ModelProfile, ProfileCandidate)
- `crates/apxm-runtime/src/model_router/profile_registry.rs` — loads TOML, validates candidates
- `crates/apxm-runtime/src/model_router/profile_router.rs` — selects healthy candidate
- `crates/apxm-compiler/src/passes/validate_model_profile.rs` — compile-time validation
- `crates/apxm-compiler/src/passes/validate_model_allowlist.rs` — allowlist enforcement (from Plan A)

**Files to modify**:
- `crates/apxm-core/src/config.rs` — add `ModelsConfig` with allowlist
- `crates/apxm-runtime/src/model_router/mod.rs` — add `select_from_profile()` and `is_model_healthy()`
- `crates/apxm-compiler/src/passes/mod.rs` — wire both validation passes
- `crates/apxm-ais/src/definitions.rs` — add `model_profile: Option<String>` to ASK/THINK/REASON attributes

## Decision Matrix

| Criterion | Plan A | Plan B |
|-----------|--------|--------|
| **Scope** | Governance only | Governance + abstraction |
| **Ship time** | 1-2 days | 1 week |
| **Backward compat** | ✅ Full (opt-in) | ✅ Full (opt-in) |
| **Graph portability** | ❌ Still hardcoded models | ✅ Semantic profiles |
| **Complexity** | Low (1 pass, 1 config field) | Medium (3 modules, 2 passes) |
| **Runtime overhead** | None | Minimal (priority sort + health check) |
| **Solves governance** | ✅ | ✅ |
| **Solves capability abstraction** | ❌ | ✅ |
| **Solves health routing** | ❌ (already done) | ❌ (already done) |

**Recommendation**: Implement Plan A immediately, evaluate Plan B based on user demand for capability abstraction.

## Examples

### Example 1: Governance Only (Plan A)

**Config** (`~/.apxm/config.toml`):
```toml
[models]
allowlist = ["claude-sonnet-4-5@20250929", "gpt-4o-mini"]
```

**Graph**:
```json
{
  "nodes": [
    {"id": 1, "op": "ASK", "model": "claude-sonnet-4-5@20250929", ...}
  ]
}
```

**Compilation**:
- ✅ Pass: model in allowlist
- If graph used `"model": "claude-opus-4-6"` → ❌ Fail: not in allowlist

### Example 2: Full Profiles (Plan B)

**Profile** (`~/.apxm/model_profiles.toml`):
```toml
[[profile]]
name = "research-tier"
candidates = [
  {model = "claude-opus-4-6", priority = 1},
  {model = "gpt-4o", priority = 2}
]
```

**Graph**:
```json
{
  "nodes": [
    {"id": 1, "op": "ASK", "model_profile": "research-tier", ...}
  ]
}
```

**Runtime**:
1. ProfileRouter loads "research-tier" profile
2. Tries "claude-opus-4-6" (priority 1) → circuit breaker CLOSED → ✅ use
3. If step 2 fails (OPEN breaker) → tries "gpt-4o" (priority 2)

### Example 3: Hybrid (Plan B with allowlist)

**Config** (`~/.apxm/config.toml`):
```toml
[models]
allowlist = ["claude-opus-4-6", "gpt-4o"]
```

**Profile** (`~/.apxm/model_profiles.toml`):
```toml
[[profile]]
name = "research-tier"
candidates = [
  {model = "claude-opus-4-6", priority = 1},
  {model = "gemini-2.0-pro", priority = 2}  # ❌ Not in allowlist!
]
```

**Compilation**:
- ❌ Fail: candidate "gemini-2.0-pro" not in allowlist
- Fix: Either add gemini to allowlist OR remove from profile candidates

## Non-Goals

### ❌ Compile-Time Health Checks
**Why**: Network failures, stale cache, 8-second build penalty. Health is runtime concern.

**Wrong approach**:
```rust
// DON'T DO THIS
fn validate_profile(profile: &ModelProfile) -> Result<()> {
    for candidate in &profile.candidates {
        let health = amd_api.check_model_health(&candidate.model)?; // ❌ Network call!
        if health.status != "available" {
            return Err(...);
        }
    }
}
```

**Right approach**: Runtime `ProfileRouter` queries `ModelRouter` circuit breakers (in-memory, millisecond latency).

### ❌ Health Data in Artifacts
**Why**: Breaks portability. A compiled artifact must run in any environment with matching backends.

**Wrong approach**:
```rust
// DON'T DO THIS
struct CompiledNode {
    model_profile: String,
    resolved_model: String,  // ❌ Baked at compile time!
}
```

**Right approach**: Artifact contains `model_profile` name; runtime resolves to model.

### ❌ Profile Discovery from Enterprise API
**Why**: Couples compilation to external API availability; breaks offline workflows.

**Wrong approach**:
```bash
# DON'T DO THIS
$ dekk apxm compile graph.apxm
Fetching profiles from https://llm.example.com... ❌
```

**Right approach**: Profiles defined in local `~/.apxm/model_profiles.toml`.

### ❌ ModelEntry "registered" or "amd_id" Fields
**Why**: No compile-time registry. ModelRegistry is runtime-only, populated from backends config.

**Wrong approach**:
```rust
// DON'T DO THIS
struct ModelEntry {
    name: String,
    registered: bool,     // ❌ What does "registered" mean at runtime?
    amd_id: String,       // ❌ vendor-specific, breaks backend abstraction
}
```

**Right approach**: ModelEntry remains simple (name, context_window, tags). Health/availability tracked in circuit breakers.

### ❌ Profile Inheritance
**Why**: Adds complexity without clear use case. Flat list of profiles is sufficient.

**Wrong approach**:
```toml
[[profile]]
name = "base-reasoning"
candidates = [...]

[[profile]]
name = "extended-reasoning"
inherits = "base-reasoning"  # ❌ Not needed
candidates = [...]
```

**Right approach**: Each profile is self-contained. Copy common candidates if needed.

## Open Questions

1. **Profile versioning**: Should profiles have versions? (e.g., `reasoning-tier@v2`)
   - **Defer**: Profiles are local config, not distributed artifacts. Version control comes from git.

2. **Profile validation**: Should compiler validate constraint fields (min_context_window)?
   - **Defer**: Constraints are hints for documentation. Validation adds complexity without clear enforcement policy.

3. **Profile UI**: Should `apxm profile list/show/validate` commands exist?
   - **Defer**: Start with manual TOML editing. Add commands based on user demand.

4. **Backend-specific profiles**: Should profiles specify backend requirements?
   - **No**: Backend routing is ModelRouter's job. Profiles are semantic, not infrastructure.

## Summary

**Plan A** provides immediate governance via allowlist enforcement. Low risk, high value for regulated environments.

**Plan B** adds capability abstraction, enabling portable graphs that adapt to runtime model availability. Higher complexity, but solves semantic modeling and improves long-term maintainability.

**Recommendation**: Ship Plan A this week. Evaluate Plan B based on feedback from users managing multiple environments or frequently rotating models.
