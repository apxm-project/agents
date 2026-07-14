# apxm-artifact

Binary artifact format for compiled APXM programs.

## Overview

`apxm-artifact` handles serialization and deserialization of compiled `.apxmobj` files. Artifacts are binary containers holding one or more `ExecutionDag` instances (multi-DAG support for multi-flow agents), metadata, optional sections, and a BLAKE3 integrity hash.

## Wire Format (52-byte header)

| Offset | Size | Field | Description |
|--------|------|-------|-------------|
| 0 | 4 | Magic | `b"APXM"` |
| 4 | 4 | Version | `1` (little-endian u32) |
| 8 | 8 | Payload length | Byte count of the payload section (little-endian u64) |
| 16 | 32 | BLAKE3 hash | Digest of the payload bytes |
| 48 | 4 | Flags | Reserved (little-endian u32, currently 0) |

The payload is a bincode-serialized `ArtifactPayload` containing `ArtifactMetadata`, one or more `WireDag`s, and optional `ArtifactSection`s. On `from_bytes()`, BLAKE3 is recomputed to reject corruption/tampering.

## Multi-DAG Support

An artifact can contain multiple DAGs for multi-flow agent definitions:

- `entry_dag()` -- returns the `@entry` DAG (first with `is_entry=true`)
- `flow_dags()` -- returns non-entry DAGs for flow registration
- `into_entry_dag()` -- consumes the artifact and returns the explicit `@entry` DAG

## Optional Sections

Artifacts may also carry opaque extension sections. `apxm-artifact` only stores
and retrieves the raw bytes; higher-level crates own any schema or parsing.

- `section_data(kind)` returns the first section payload for a kind without
  decoding it.
- `replace_section(kind, data)` inserts or updates a section while preserving
  the bytes exactly as provided.

## Module Structure

| Module | Description |
|--------|-------------|
| `lib` | `Artifact`, `ArtifactMetadata`, `ArtifactSection`, serialization/deserialization |
| `wire` | `WireDag` -- bincode-friendly wire representation of `ExecutionDag` |

## Key Exports

- `Artifact` -- main artifact container (multi-DAG)
- `ArtifactMetadata` -- module name, creation timestamp, compiler version
- `ArtifactSection` -- optional extension sections
- `ArtifactError` -- error type for I/O, version mismatch, and hash failures

## Usage

```rust
use apxm_artifact::Artifact;

// Read
let artifact = Artifact::read_from_path("program.apxmobj")?;
let entry = artifact.entry_dag().expect("no entry DAG");

// Write
artifact.write_to_path("output.apxmobj")?;

// Hash verification
let hash = artifact.payload_hash()?;
```

## Dependencies

| Crate | Purpose |
|-------|---------|
| apxm-core | `ExecutionDag`, `Node`, `Edge` types |
