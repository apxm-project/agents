# APXM File Format Design

*What the best projects do, and what APXM should do.*

---

## Current State

APXM has three file types today:

| Extension | What it is | Format |
|-----------|-----------|--------|
| `.ais` | Agent source language | Text (custom DSL) |
| `.apxm` | Graph IR (hand-written) | JSON |
| `.apxmobj` | Compiled artifact | Custom binary |

The `.apxmobj` format is already well-designed:
- Magic bytes: `b"APXM"` 
- Version: u32
- Payload length: u64
- BLAKE3 hash: 32 bytes (integrity)
- Flags: u32
- Payload: bincode-serialized `ArtifactPayload`

The `.apxm` JSON is the **intermediate representation** — currently written by hand. With the `.ais` parser, nobody writes it by hand anymore. It becomes a compiler-internal format.

---

## How the Best Projects Do It

### LLVM: Three-tier format system

```
foo.c      → source (C)
foo.ll     → LLVM IR (text, human-readable)
foo.bc     → LLVM bitcode (binary IR)
foo.o      → object file (native binary)
```

Key insight: **Two IR formats** — text (`.ll`) for humans/debugging, binary (`.bc`) for toolchains. The text IR is the canonical debugging format. The binary IR is what gets linked and shipped.

### ONNX: Protobuf for the graph IR

```
model.py        → source (Python/PyTorch)
model.onnx      → serialized graph (protobuf binary)
model.ort       → optimized runtime format (ORT)
```

ONNX uses protobuf for the graph IR. Why: schema enforcement, versioning, language-agnostic, compact. The `.onnx` file IS the graph + weights + metadata. Inspectable with `onnx.load()`. Not hand-written.

### PyTorch: Two-format strategy

```
model.pt / model.pth    → pickle (Python-only, legacy)
model.pt2               → TorchScript (portable, compilable)
```

PyTorch moved away from pickle toward structured formats because pickle is Python-only and not forward-compatible.

### WebAssembly: Binary + text pair

```
foo.wat     → WebAssembly text format (S-expressions, human-readable)
foo.wasm    → WebAssembly binary (compact, validated, executable)
```

The `.wat` text format exists purely for human inspection and debugging. The `.wasm` binary is what runs. They're semantically equivalent — any `.wat` compiles to `.wasm`.

### Protocol Buffers (Google)

```
foo.proto   → schema definition
foo.pb      → serialized binary
foo.pbtxt   → text format (for debugging)
```

Three tiers: schema, binary, text debug format.

---

## What APXM Should Do

The answer is clear from studying these systems. APXM needs exactly **three tiers**:

### Tier 1: Source — `.ais`
The agent authoring language. Human-written. Compiled by the AIS parser.
- Already designed and implemented
- One `.ais` file = one or more agent definitions
- The `.ais` → graph lowering happens in the compiler

### Tier 2: IR — `.apxmir` (not `.apxm`)
The intermediate representation. **Not JSON.** Should be:

**Option A: Binary (flatbuffers/bincode)** — fast, compact, not human-readable
**Option B: Text IR (like LLVM `.ll`)** — human-readable, debuggable, inspectable
**Option C: Both** — binary for toolchains, text for debugging ← **best choice**

The current `.apxm` JSON is essentially the text IR but with JSON syntax, which is:
- ✅ Human-readable 
- ✅ Inspectable with any editor
- ❌ Verbose (graph with 20 nodes = 200+ lines)
- ❌ No schema enforcement
- ❌ Slow to parse at scale
- ❌ Not a professional format

**The recommendation:** Keep JSON as the text IR (rename to `.apxmir`) for debugging. Add a binary IR (flatbuffers) for toolchain use. The compiler reads either.

### Tier 3: Artifact — `.apxmobj`
Already well-designed. Keep it. BLAKE3 hash, magic bytes, bincode payload.
Possibly rename to `.apxa` (shorter, cleaner) — like `.o` for object files.

---

## Concrete Recommendation

```
Source:      agent.ais         → AIS source language (text, human-written)
Text IR:     agent.apxmir      → Graph IR, text format (JSON or S-expr, debuggable)
Binary IR:   agent.apxmb       → Graph IR, binary (flatbuffers, toolchain use)
Artifact:    agent.apxmobj     → Compiled binary (keep current format)
```

Or more concisely, mirroring LLVM:

```
.ais        → source    (like .c / .cpp)
.apxmir     → text IR   (like .ll)
.apxmb      → binary IR (like .bc)  ← NEW
.apxmobj    → artifact  (like .o)
```

### Why flatbuffers for `.apxmb`?

| Format | Why |
|--------|-----|
| JSON | Human-readable but slow, verbose, no schema |
| bincode | Fast but Rust-only, no schema |
| protobuf | Schema, versioned, but complex |
| **flatbuffers** | Zero-copy, schema, fast, language-agnostic, used by TFLite |
| MessagePack | Compact but no schema |

Flatbuffers wins because:
1. Zero-copy reads — load a 10MB agent graph in microseconds
2. Schema enforced at build time (`.fbs` schema file)
3. Language-agnostic — C++, Rust, Python can all read it
4. Forward/backward compatible with table evolution
5. Used by TensorFlow Lite for exactly this use case (ML graph serialization)

---

## Migration Path

**Phase 1 (now):** Keep current formats, wire `.ais` → compiler (the 10-line fix)

**Phase 2:** Add `.apxmb` flatbuffers binary IR alongside JSON
- Define `.fbs` schema for `ApxmGraph`
- Generate Rust + C++ bindings
- The driver accepts both `.apxm` (JSON) and `.apxmb` (binary)

**Phase 3:** `.apxm` JSON becomes the legacy/debug format
- Production toolchains use `.apxmb`
- `.apxm` still works for human inspection and debugging

**Phase 4 (optional):** Rename extensions for clarity
- `.apxmir` for the text IR (signals it's an IR, not source)

---

## The Extension Question: `.apxm` for the graph

Why `.apxm` for the graph IR is confusing:
- `.apxm` sounds like the main APXM format
- But it's actually the intermediate representation, not source and not artifact
- Users see three files ending in `.apxm`, `.apxmobj` and don't know which to write vs which is compiler output

Better mental model (LLVM-style):

```
You write:    agent.ais       ← the agent you author
You get:      agent.apxmobj   ← the compiled artifact you run
You can inspect: agent.apxmir ← the IR, if you need to debug
```

The IR is a compiler-internal detail. Expose it for debugging, not for authoring.

---

## What This Means for APXM Right Now

1. **Immediate:** Wire `.ais` → `Module::parse_dsl_file()` in the driver (10 lines)
2. **Soon:** Add `.fbs` schema for `ApxmGraph`, generate flatbuffers bindings
3. **Later:** Rename `.apxm` → `.apxmir` in docs and examples to clarify it's IR
4. **The `.apxmobj` format is already correct** — binary, magic bytes, BLAKE3, versioned. Keep it.

The current JSON format is fine to keep as the human-readable IR. The addition of a binary IR format is the professional step — it's what makes APXM usable in production toolchains where parsing 10,000-node graphs from JSON is too slow.

---

## Current Implementation Status (2026-04-05)

### What's implemented

```
.ais    → source language — WRITE THIS (human and AgentMate)
.air    → inspection/debug dump — READ ONLY (output of 'emit-ir', not round-trip)
.apxmobj → compiled binary artifact — EXECUTE THIS (output of 'compile')
.apxm   → REMOVED (was legacy JSON graph format, no longer accepted)
```

### The pipeline

```
AgentMate (Rust/Python API) ──→ emits .ais source
Human author               ──→ writes .ais source
                                    │
                                    ▼
                          dekk apxm execute <file.ais>
                          (GraphGen: DSL→ApxmGraph, graph-direct execution)
                                    │
                                    ▼
                                  result
```

For compiled artifacts (production):
```
.ais → dekk apxm compile → .apxmobj → dekk apxm run → result
```

For inspection:
```
.ais → dekk apxm execute (internally emits .air to session dir for debugging)
```

### Why .air is output-only

The `.air` format in examples/ is an abbreviated human-readable dump, not valid
round-trip MLIR text. Full MLIR text requires module/function wrappers and
type annotations that the abbreviated dump omits. Making `.air` a round-trip
format requires either: (a) fixing `emit_air` to produce full valid MLIR, or
(b) writing a `.air` → `ApxmGraph` parser. This is P2 work.

### AgentMate's role

AgentMate is the Clang to APXM's LLVM. It provides:
- Rust builder API (`AgentBuilder`, `WorkflowBuilder`)
- Python API (`@compile` decorator, `ag.ask()`, `ag.think()`)

AgentMate should **emit `.ais` files** (not `.air`). The `.ais` DSL is the
canonical input format. AgentMate is a frontend that makes writing `.ais` easier.
