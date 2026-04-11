# Getting Started with APXM

APXM (Agent Program Execution Model) is a compiler and runtime for AI agent
workflows. You write workflows in Python, the compiler lowers them to AIR
(Agent Intermediate Representation), and the runtime executes them against
LLM backends.

## Prerequisites

- Python 3.10+
- Rust nightly
- conda or mamba
- An LLM backend: cloud API key (OpenAI, Anthropic), enterprise gateway, or local (Ollama, vLLM)

## Installation

```bash
dekk apxm install      # Sets up conda env + builds from source
dekk apxm doctor       # Verify environment health
```

## Register a Backend

APXM needs at least one LLM backend to execute workflows. Backends are stored
in `~/.apxm/config.toml` with 0o600 permissions (owner-only, like `~/.ssh/config`).
Keys are masked in `backend list` output. A `.gitignore` is auto-created in
`~/.apxm/` to prevent committing credentials.

### Cloud (OpenAI / Anthropic)

```bash
# Interactive prompt — input hidden via rpassword
dekk apxm backend add openai --type cloud --protocol openai
# → "Enter API key for openai (or press Enter to skip): ****"

dekk apxm backend add anthropic --type cloud --protocol anthropic
```

Never pass `--api-key sk-...` directly on the command line — it's visible in
shell history. Use the interactive prompt instead.

If your API key is already in your environment (e.g., set by your shell
profile), you can reference it without exposing the value:

```toml
# In ~/.apxm/config.toml — resolves at runtime, not stored in file
[[backends]]
name = "openai"
type = "cloud"
protocol = "openai"
api_key = "env:OPENAI_API_KEY"    # Reads $OPENAI_API_KEY at startup
```

### Enterprise Gateway (on-prem, header-based auth)

For corporate API gateways that serve multiple model families behind a single
endpoint. Auth is typically via subscription key headers, not API keys:

```bash
dekk apxm backend add my-gateway \
  --type onprem \
  --protocol openai \
  --endpoint https://llm-api.example.com/api \
  --api-key dummy \
  --header Ocp-Apim-Subscription-Key=YOUR_KEY
```

- `--protocol openai` means "speaks OpenAI-compatible API format" — it can serve any model
- `--api-key dummy` — placeholder; real auth goes through the subscription header
- `--header Key=Value` — custom HTTP headers sent with every request
- Headers support `env:` references: `--header X-Auth=env:MY_SECRET`

### Local (Ollama)

```bash
dekk apxm backend add ollama --protocol ollama
# Auto-detects: type=local, endpoint=http://localhost:11434
# Auto-syncs installed models via Ollama API
```

### Local vLLM (GPU inference)

```bash
dekk apxm backend add vllm-local --type local --protocol openai \
  --endpoint http://localhost:8000 --api-key dummy
```

### Verify

```bash
dekk apxm backend list    # Shows backends with masked keys (sk-p...xyz)
dekk apxm backend test    # Validates connectivity to all backends
```

### Credential Security Summary

| Feature | Detail |
|---------|--------|
| Storage | `~/.apxm/config.toml` (plaintext, like ~/.ssh/config) |
| Permissions | 0o600 (owner read/write only) — enforced on every read |
| Atomic writes | tempfile + rename (prevents corruption) |
| Key masking | `backend list` shows `sk-p...xyz` (first 4 + last 4) |
| Env var indirection | `api_key = "env:VAR"` resolves at runtime |
| .gitignore | Auto-created in ~/.apxm/ |
| Keychain/vault | Not integrated — standard file-based credentials |

## Register Models

Models describe what a backend can serve. Required for routing.

### Via CLI

```bash
dekk apxm backend add-model my-gateway claude-sonnet-4-5 \
  --alias smart --alias balanced \
  --context-window 200000 \
  --supports-vision --supports-functions \
  --tag anthropic --tag production

dekk apxm backend add-model my-gateway gpt-4o-mini \
  --alias fast --alias cheap \
  --context-window 128000 \
  --supports-functions \
  --tag openai --tag fast
```

Flags: `--alias` (routing names), `--context-window`, `--supports-vision`,
`--supports-functions`, `--supports-thinking`, `--tag`, `--cost-input`,
`--cost-output`.

### Via config.toml

More common for bulk setup:

```toml
[[backends.models]]
id = "claude-sonnet-4-5@20250929"
aliases = ["claude", "sonnet", "smart"]
context_window = 200000
supports_vision = true
supports_functions = true
tags = ["anthropic", "balanced"]
```

### Ollama auto-sync

```bash
dekk apxm backend sync-models ollama
# Queries Ollama API, discovers installed models + capabilities
```

## Configure Routing (optional but recommended)

The routing hierarchy: per-node `model=` attribute > operation route > default model > first backend.

### Defaults

```toml
[chat]
providers = ["my-gateway"]          # Which backends to activate
default_backend = "my-gateway"
default_model = "claude-sonnet-4-5"
```

### Per-operation routing

```toml
[chat.routing.operation_routes.ask]
backend = "my-gateway"
model = "gpt-4o-mini"            # Fast/cheap for simple Q&A

[chat.routing.operation_routes.think]
backend = "my-gateway"
model = "claude-sonnet-4-5"      # Powerful for deep reasoning

[chat.routing.operation_routes.verify]
backend = "my-gateway"
model = "o4-mini"                # Reasoning model for fact-checking
```

### Model aliases

Runtime routing shortcuts:

```toml
[chat.routing.model_aliases.smart]
model = "claude-sonnet-4-5"
backend = "my-gateway"

[chat.routing.model_aliases.fast]
model = "gpt-4o-mini"
backend = "my-gateway"

[chat.routing.model_aliases.reasoner]
model = "o4-mini"
backend = "my-gateway"
```

### Fallback chains

```toml
[[chat.routing.fallback_chains]]
backend = "primary"
fallbacks = ["secondary", "tertiary"]
```

### Using models in workflows

APXM provides typed model constants via `_generated/models.py`:

```python
from apxm import Anthropic, OpenAI, Google

# Typed model selection — name is optional, auto-generated if omitted
g.ask("Summarize this document", model=Anthropic.CLAUDE_SONNET_4)
g.think("Analyze the architecture deeply", model=Anthropic.CLAUDE_OPUS_4)
g.ask("Quick summary", model=OpenAI.GPT_4O_MINI)
g.verify(claim="The sky is blue", model=Google.GEMINI_2_5_FLASH)

# Each provider class has a DEFAULT:
g.ask("Hello world", model=Anthropic.DEFAULT)  # claude-sonnet-4-5
```

Model aliases defined in config.toml are resolved at runtime — the Python
frontend doesn't need to know about them. Per-operation routing is also
applied at runtime (e.g., all ASK nodes get haiku, THINK nodes get sonnet).

**Custom/dynamic models** (fine-tuned, self-hosted, etc.) are registered as
backend models with aliases in config.toml — never hardcoded as raw strings
in Python:

```toml
# Register your custom model in config.toml
[[backends.models]]
id = "my-org/fine-tuned-summarizer-v3"
aliases = ["summarizer"]
context_window = 32000

[chat.routing.model_aliases.summarizer]
model = "my-org/fine-tuned-summarizer-v3"
backend = "vllm-local"
```

The runtime routing system resolves the model at execution time — no raw
string model IDs in workflow code. If you omit `model=` entirely, the
per-operation route or default model from config handles it.

## Your First Workflow

```python
from apxm import compile, GraphRecorder

@compile()
def hello_world(g: GraphRecorder):
    greeting = g.ask("greeting",
        "Generate a friendly greeting for someone learning about AI agents")
    g.done(greeting)
```

Line-by-line:
- `from apxm import compile, GraphRecorder` — core framework imports
- `@compile()` — decorator that records the function body as a graph
- `g.ask(name, template)` — creates an ASK node
- `g.done(node)` — marks the return value of the workflow

The `ask()` calling conventions:

```python
# Template only — name auto-generated ("ask", "ask_1", ...)
g.ask("What is the capital of France?")

# Explicit name + template
g.ask("greeting", "Generate a friendly greeting")

# Keyword args
g.ask(name="greeting", template="Generate a friendly greeting")
```

A second example with typed model selection and auto-wiring:

```python
from apxm import compile, GraphRecorder, Anthropic, OpenAI

@compile()
def model_routing_demo(g: GraphRecorder):
    # Quick triage with a fast model (name auto-generated)
    triage = g.ask(
        "Classify this topic: quantum computing",
        model=OpenAI.GPT_4O_MINI,
    )

    # Deep analysis — {triage} auto-wires a data dependency
    analysis = g.think(
        "Provide deep analysis of: {triage}",
        model=Anthropic.CLAUDE_SONNET_4,
    )

    g.done(analysis)
```

Key concepts:
- `{triage}` in a template creates an automatic data edge from the `triage` node
- `model=` accepts typed `ModelId` constants from `Anthropic`, `OpenAI`, `Google`
- If `model=` is omitted, the per-operation route from config.toml is used

## Run It

```bash
# Execute directly (compile + run in one step)
dekk apxm execute examples/python/getting-started/hello.py

# Or compile first, run later
dekk apxm compile examples/python/getting-started/hello.py -o hello.apxmobj
dekk apxm run hello.apxmobj

# With session tracing (see all intermediate outputs)
dekk apxm execute examples/python/getting-started/hello.py --emit-session
```

## Docker-Managed Backends

For backends with Docker configs:

```bash
dekk apxm backend start vllm-local    # docker run ...
dekk apxm backend status vllm-local   # check container
dekk apxm backend logs vllm-local --tail 50
dekk apxm backend stop vllm-local
dekk apxm backend restart vllm-local
```

## Troubleshooting

| Error | Cause | Fix |
|-------|-------|-----|
| "No backends configured" | No `[[backends]]` in config.toml | `dekk apxm backend add ...` |
| "No backends registered" | Backends in config but not loaded | Check `[chat] providers = [...]` includes your backend |
| Backend test fails | Wrong endpoint/key | Check URL and credentials; for on-prem check headers |
| "Model not found" | Model not registered on backend | `dekk apxm backend add-model ...` |
| Python ImportError | Codegen not run | `dekk apxm codegen frontend` |

## Next Steps

- `examples/python/getting-started/tool_use.py` — Tool invocation
- `examples/python/patterns/` — Iterative refinement, cross-critique, resilient pipelines
- `examples/python/multi-agent/` — Multi-agent coordination and teams
- `dekk apxm ops list` — Browse all 41 AIS operations
- `dekk apxm template list` — Starter templates
- `dekk apxm analyze <graph>` — Parallelism analysis
- `docs/README.md` — Full documentation index

## Reference: config.toml

Full annotated skeleton:

```toml
# ---- BACKENDS ---- where to send LLM requests

[[backends]]
name = "my-gateway"                           # Unique identifier
type = "onprem"                               # cloud | onprem | local
protocol = "openai"                           # openai | anthropic | ollama | vllm
endpoint = "https://llm-api.example.com/api"  # API base URL
api_key = "env:MY_API_KEY"                    # env:VAR or literal (0o600 protected)

[backends.headers]                            # Custom HTTP headers
Ocp-Apim-Subscription-Key = "env:SUB_KEY"     # Supports env: resolution
user = "env:USER"                             # OS username at runtime

# ---- Models hosted on this backend ----

[[backends.models]]
id = "claude-sonnet-4-5"                      # Model ID sent to API
aliases = ["claude", "sonnet", "smart"]       # Routing shortcuts
context_window = 200000                       # Max context in tokens
supports_vision = true                        # Image input capability
supports_functions = true                     # Tool/function calling
supports_thinking = false                     # Extended reasoning
tags = ["anthropic", "balanced"]              # Classification tags

[[backends.models]]
id = "gpt-4o-mini"
aliases = ["fast", "cheap"]
context_window = 128000
supports_functions = true
tags = ["openai", "fast"]

# ---- ROUTING ---- how to select models for operations

[chat]
providers = ["my-gateway"]                    # Which backends to activate
default_backend = "my-gateway"                # Fallback backend
default_model = "claude-sonnet-4-5"           # Fallback model
planning_model = "claude-sonnet-4-5"          # For PLAN operations

# Per-operation model assignment
[chat.routing.operation_routes.ask]
backend = "my-gateway"
model = "gpt-4o-mini"                         # Fast/cheap for Q&A

[chat.routing.operation_routes.think]
backend = "my-gateway"
model = "claude-sonnet-4-5"                   # Powerful for reasoning

[chat.routing.operation_routes.verify]
backend = "my-gateway"
model = "o4-mini"                             # Reasoning for fact-check

# Named aliases (used in workflows and routing)
[chat.routing.model_aliases.smart]
model = "claude-sonnet-4-5"
backend = "my-gateway"

[chat.routing.model_aliases.fast]
model = "gpt-4o-mini"
backend = "my-gateway"

# Fallback chains (if primary backend fails)
# [[chat.routing.fallback_chains]]
# backend = "primary"
# fallbacks = ["secondary"]

# Per-operation system prompt overrides
[chat.instructions]
# think = "You are a deep reasoning expert..."
# ask = "You are a concise, helpful assistant..."
```

Key points:
- All `api_key`, `endpoint`, and header values support `env:VAR_NAME` indirection
- File is 0o600 (owner-only) — APXM validates permissions on every read
- `providers` list controls which `[[backends]]` are activated (whitelist)
- Routing priority: per-node `model=` attribute > operation route > default model > first backend
