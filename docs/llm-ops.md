# LLM Operations — Typed Model Infrastructure

**Status:** Plan (replaces previous design draft)
**Scope:** apxm-core (SSOT) + codegen pipeline + Python frontend

---

## Architecture: Single Source of Truth in apxm-core

```
apxm-core (defines WellKnownModel registry)
    |
    +---> apxm-backends (consumes: validates, routes, defaults)
    +---> apxm-compiler (consumes: latency markers, scheduling)
    +---> codegen pipeline (apxm codegen frontend)
              |
              +---> _generated/models.py (typed Python constants)
                        |
                        +---> proxy.py (ask/think/reason/plan/reflect/verify)
```

**Principle:** Backends register to APXM. Models register to backends. apxm-core
exports everything. Downstream consumers (runtime, compiler, frontend) all read
from the same source. Any model works with any LLM operation.

---

## Phase 1: Central Model Registry in apxm-core

### What changes

Create `apxm-core/src/types/models/well_known.rs` — the single source of truth
for all backend model definitions.

### New types

```rust
// apxm-core/src/types/models/well_known.rs

/// A model known to a specific backend.
pub struct WellKnownModel {
    /// Model identifier string (e.g. "claude-sonnet-4-5")
    pub id: &'static str,
    /// Whether this is the backend's default model
    pub default: bool,
}

/// All well-known models for a given backend.
pub struct BackendModels {
    /// Backend name (e.g. "anthropic")
    pub backend: &'static str,
    /// Wire protocol
    pub protocol: ProviderProtocol,
    /// Known model catalog
    pub models: &'static [WellKnownModel],
}
```

### Constant definitions

```rust
pub const ANTHROPIC_MODELS: BackendModels = BackendModels {
    backend: "anthropic",
    protocol: ProviderProtocol::Anthropic,
    models: &[
        WellKnownModel { id: "claude-opus-4-5",              default: false },
        WellKnownModel { id: "claude-sonnet-4-5",            default: true  },
        WellKnownModel { id: "claude-3-7-sonnet",            default: false },
        WellKnownModel { id: "claude-3-5-sonnet-20241022",   default: false },
        WellKnownModel { id: "claude-3-opus-20240229",       default: false },
        WellKnownModel { id: "claude-3-sonnet-20240229",     default: false },
        WellKnownModel { id: "claude-3-haiku-20240307",      default: false },
    ],
};

pub const OPENAI_MODELS: BackendModels = BackendModels {
    backend: "openai",
    protocol: ProviderProtocol::OpenAI,
    models: &[
        WellKnownModel { id: "gpt-4o",          default: false },
        WellKnownModel { id: "gpt-4o-mini",     default: true  },
        WellKnownModel { id: "gpt-4-turbo",     default: false },
        WellKnownModel { id: "gpt-4",           default: false },
        WellKnownModel { id: "gpt-3.5-turbo",   default: false },
        WellKnownModel { id: "gpt-5",           default: false },
        WellKnownModel { id: "gpt-5-mini",      default: false },
        WellKnownModel { id: "gpt-5-nano",      default: false },
        WellKnownModel { id: "gpt-5.1",         default: false },
        WellKnownModel { id: "gpt-5.2",         default: false },
        WellKnownModel { id: "o1",              default: false },
        WellKnownModel { id: "o1-mini",         default: false },
        WellKnownModel { id: "o1-preview",      default: false },
    ],
};

pub const GOOGLE_MODELS: BackendModels = BackendModels {
    backend: "google",
    protocol: ProviderProtocol::Google,
    models: &[
        WellKnownModel { id: "gemini-2.5-flash", default: true  },
        WellKnownModel { id: "gemini-2.0-pro",   default: false },
        WellKnownModel { id: "gemini-1.5-pro",   default: false },
        WellKnownModel { id: "gemini-1.5-flash",  default: false },
    ],
};

/// Dynamic backends — no static model catalog.
/// Users register models via `apxm backend add-model`.
pub const OLLAMA_MODELS: BackendModels = BackendModels {
    backend: "ollama",
    protocol: ProviderProtocol::Ollama,
    models: &[],
};

pub const VLLM_MODELS: BackendModels = BackendModels {
    backend: "vllm",
    protocol: ProviderProtocol::Vllm,
    models: &[],
};

/// All backend model registries, iterable by codegen and runtime.
pub const ALL_BACKEND_MODELS: &[&BackendModels] = &[
    &ANTHROPIC_MODELS,
    &OPENAI_MODELS,
    &GOOGLE_MODELS,
    &OLLAMA_MODELS,
    &VLLM_MODELS,
];
```

### apxm-core exports

Add to `apxm-core/src/types/models/mod.rs`:

```rust
mod well_known;
pub use well_known::*;
```

Re-export from `apxm-core/src/lib.rs` so all 11 downstream crates get it:

```rust
pub use types::models::{WellKnownModel, BackendModels, ALL_BACKEND_MODELS};
pub use types::models::{ANTHROPIC_MODELS, OPENAI_MODELS, GOOGLE_MODELS, OLLAMA_MODELS, VLLM_MODELS};
```

### Files touched

| File | Change |
|------|--------|
| `apxm-core/src/types/models/well_known.rs` | **NEW** — central registry |
| `apxm-core/src/types/models/mod.rs` | **EDIT** — add `mod well_known; pub use well_known::*;` |
| `apxm-core/src/lib.rs` | **EDIT** — re-export new types |

---

## Phase 2: Migrate Backends to Consume from apxm-core

### What changes

Each backend's `model.rs` stops defining its own `WELL_KNOWN_MODELS` and
`ModelId` newtype. Instead it imports from `apxm-core`.

### Before (per-backend duplication)

```rust
// apxm-backends/src/llm/backends/anthropic/model.rs (CURRENT)
pub struct ModelId(pub String);  // duplicated 3x
pub const WELL_KNOWN_MODELS: &[&str] = &[...];  // duplicated 3x
```

### After (single import from core)

```rust
// apxm-backends/src/llm/backends/anthropic/model.rs (NEW)
use apxm_core::types::models::{ANTHROPIC_MODELS, WellKnownModel};
pub use apxm_core::ModelId;  // single definition in core

pub fn well_known_models() -> &'static [WellKnownModel] {
    ANTHROPIC_MODELS.models
}

pub fn default_model() -> &'static str {
    ANTHROPIC_MODELS.models.iter()
        .find(|m| m.default)
        .map(|m| m.id)
        .unwrap_or("claude-sonnet-4-5")
}
```

### Files touched

| File | Change |
|------|--------|
| `apxm-backends/.../anthropic/model.rs` | **EDIT** — import from core, remove local defs |
| `apxm-backends/.../openai/model.rs` | **EDIT** — import from core, remove local defs |
| `apxm-backends/.../google/model.rs` | **EDIT** — import from core, remove local defs |

---

## Phase 3: Extend Codegen Pipeline

### What changes

The existing `apxm codegen frontend` pipeline gets a new module: `_generated/models.py`.
This follows the exact same pattern as `_generated/constants.py` and `_generated/operations.py`.

### Codegen implementation

```rust
// apxm-cli/src/frontend/registry.rs — NEW function

pub struct FrontendModelSpec {
    pub backend: String,          // "anthropic"
    pub constant_name: String,    // "CLAUDE_SONNET_4"
    pub model_id: String,         // "claude-sonnet-4-5"
    pub is_default: bool,
}

pub fn model_specs() -> Vec<(String, Vec<FrontendModelSpec>)> {
    apxm_core::ALL_BACKEND_MODELS
        .iter()
        .map(|bm| {
            let specs = bm.models.iter().map(|m| FrontendModelSpec {
                backend: bm.backend.to_string(),
                constant_name: to_python_constant(m.id),  // "claude-sonnet-4-5" → "CLAUDE_SONNET_4"
                model_id: m.id.to_string(),
                is_default: m.default,
            }).collect();
            (bm.backend.to_string(), specs)
        })
        .collect()
}
```

```rust
// apxm-cli/src/frontend/codegen.rs — NEW function

fn render_models_module(specs: &[(String, Vec<FrontendModelSpec>)]) -> String {
    // Generates:
    //   class ModelId: ...
    //   class Anthropic: CLAUDE_OPUS_4 = ModelId("claude-opus-4-5") ...
    //   class OpenAI: GPT_4O = ModelId("gpt-4o") ...
    //   class Google: GEMINI_2_5_FLASH = ModelId("gemini-2.5-flash") ...
}
```

### Generated output

```python
# _generated/models.py — AUTO-GENERATED by `apxm codegen frontend`

from typing import Final

class ModelId:
    """Typed model identifier. Converts to str for graph JSON serialization."""
    __slots__ = ("_value",)
    def __init__(self, value: str) -> None:
        self._value = value
    def __str__(self) -> str:
        return self._value
    def __repr__(self) -> str:
        return f"ModelId({self._value!r})"
    def __eq__(self, other: object) -> bool:
        if isinstance(other, ModelId):
            return self._value == other._value
        if isinstance(other, str):
            return self._value == other
        return NotImplemented
    def __hash__(self) -> int:
        return hash(self._value)


class Anthropic:
    """Anthropic backend models (protocol: anthropic)."""
    CLAUDE_OPUS_4: Final[ModelId] = ModelId("claude-opus-4-5")
    CLAUDE_SONNET_4: Final[ModelId] = ModelId("claude-sonnet-4-5")
    CLAUDE_3_7_SONNET: Final[ModelId] = ModelId("claude-3-7-sonnet")
    CLAUDE_3_5_SONNET: Final[ModelId] = ModelId("claude-3-5-sonnet-20241022")
    CLAUDE_3_OPUS: Final[ModelId] = ModelId("claude-3-opus-20240229")
    CLAUDE_3_SONNET: Final[ModelId] = ModelId("claude-3-sonnet-20240229")
    CLAUDE_3_HAIKU: Final[ModelId] = ModelId("claude-3-haiku-20240307")
    DEFAULT: Final[ModelId] = CLAUDE_SONNET_4


class OpenAI:
    """OpenAI backend models (protocol: openai)."""
    GPT_4O: Final[ModelId] = ModelId("gpt-4o")
    GPT_4O_MINI: Final[ModelId] = ModelId("gpt-4o-mini")
    GPT_4_TURBO: Final[ModelId] = ModelId("gpt-4-turbo")
    GPT_4: Final[ModelId] = ModelId("gpt-4")
    GPT_3_5_TURBO: Final[ModelId] = ModelId("gpt-3.5-turbo")
    GPT_5: Final[ModelId] = ModelId("gpt-5")
    GPT_5_MINI: Final[ModelId] = ModelId("gpt-5-mini")
    GPT_5_NANO: Final[ModelId] = ModelId("gpt-5-nano")
    GPT_5_1: Final[ModelId] = ModelId("gpt-5.1")
    GPT_5_2: Final[ModelId] = ModelId("gpt-5.2")
    O1: Final[ModelId] = ModelId("o1")
    O1_MINI: Final[ModelId] = ModelId("o1-mini")
    O1_PREVIEW: Final[ModelId] = ModelId("o1-preview")
    DEFAULT: Final[ModelId] = GPT_4O_MINI


class Google:
    """Google backend models (protocol: google)."""
    GEMINI_2_5_FLASH: Final[ModelId] = ModelId("gemini-2.5-flash")
    GEMINI_2_0_PRO: Final[ModelId] = ModelId("gemini-2.0-pro")
    GEMINI_1_5_PRO: Final[ModelId] = ModelId("gemini-1.5-pro")
    GEMINI_1_5_FLASH: Final[ModelId] = ModelId("gemini-1.5-flash")
    DEFAULT: Final[ModelId] = GEMINI_2_5_FLASH
```

### Files touched

| File | Change |
|------|--------|
| `apxm-cli/src/frontend/registry.rs` | **EDIT** — add `model_specs()` + `FrontendModelSpec` |
| `apxm-cli/src/frontend/codegen.rs` | **EDIT** — add `render_models_module()`, update `render_generated_files()` |

---

## Phase 4: Update Python Frontend

### What changes

1. `proxy.py` — model parameter accepts `str | ModelId | None`
2. `__init__.py` — re-exports `ModelId`, `Anthropic`, `OpenAI`, `Google`

### proxy.py signature change

```python
def ask(
    self,
    name_or_template: str | None = None,
    template_arg: str | None = None,
    *,
    name: str | None = None,
    template: str | None = None,
    agent: AgentConfig | None = None,
    model: str | ModelId | None = None,   # <-- accepts typed or raw string
    provider: str | None = None,
    backend: str | None = None,
    **attributes: Any,
) -> NodeRef:
```

The only logic change is in `_normalize_value()` — add `ModelId` conversion:

```python
def _normalize_value(self, value: Any) -> Any:
    if isinstance(value, ModelId):
        return str(value)
    # ... existing logic
```

### Frontend usage

```python
from apxm import Anthropic, OpenAI, Google

# Typed — IDE autocomplete catches typos at write time
result = g.ask("Generate a greeting", model=Anthropic.CLAUDE_SONNET_4)
analysis = g.think("Analyze this code", model=Anthropic.CLAUDE_OPUS_4)
plan = g.plan("Migrate to FastAPI", model=OpenAI.GPT_4O)
summary = g.reflect("Review execution", model=Google.GEMINI_2_5_FLASH)

# Raw strings still work — for custom/new/dynamic models
result = g.ask("prompt", model="my-fine-tuned-model-v3")
result = g.think("prompt", model="ollama/llama3:70b")
```

**Any model works with any LLM operation.** The 6 ops (ask, think, reason, plan,
reflect, verify) are operation semantics — they don't restrict which model you use.

### Files touched

| File | Change |
|------|--------|
| `apxm-frontend/python/apxm/proxy.py` | **EDIT** — import ModelId, update type hints, add normalize |
| `apxm-frontend/python/apxm/__init__.py` | **EDIT** — re-export ModelId, Anthropic, OpenAI, Google |

---

## Phase 5: Regenerate and Verify

```bash
# 1. Build with new core types
dekk apxm build

# 2. Regenerate Python frontend bindings
dekk apxm codegen frontend

# 3. Run full test suite
dekk apxm test

# 4. Verify generated models.py
cat crates/compiler/apxm-frontend/python/apxm/_generated/models.py
```

---

## Implementation Order

```
Phase 1 ──> Phase 2 ──> Phase 3 ──> Phase 4 ──> Phase 5
  core        backends    codegen     frontend    verify
  (SSOT)      (consume)   (export)    (import)    (test)
```

Each phase is independently buildable and testable. No phase breaks
existing functionality — raw string model parameters continue to work
throughout.

---

## What does NOT change

- **AIS operation definitions** — the 6 LLM ops are fixed in the ISA
- **LLMRegistry routing** — per-op routing, fallback chains, aliases stay as-is
- **Config hierarchy** — node attr > operation_routes > default_model > backend default
- **Graph JSON format** — model stays as a string attribute
- **Runtime validation** — models validated at API call time, not graph construction

---

## See Also

- `apxm-core/src/types/models/` — central model registry (SSOT)
- `apxm-ais/src/operations/definitions.rs` — AIS operation definitions
- `apxm-backends/src/llm/registry/mod.rs` — LLM registry with per-op routing
- `apxm-cli/src/frontend/codegen.rs` — Python codegen pipeline
- `apxm-frontend/python/apxm/proxy.py` — Python frontend (GraphRecorder)
