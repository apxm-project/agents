# apxm-server container image

This directory ships a multi-stage `Dockerfile` and a two-tenant
`docker-compose.example.yml` for running `apxm-server` in containers.
The image exists to make the multi-instance contract enforceable in
deployments — see "Why this exists" below.

## Build

From the apxm repo root:

```bash
docker build -t apxm-server:latest -f deploy/apxm-server/Dockerfile .
```

Do not run this from inside `deploy/apxm-server/` — the build context
must include the workspace (the build stage runs
`cargo build -p apxm-server`).

## Run a single instance

```bash
mkdir -p /var/lib/apxm/tenant-a
# Populate config.toml — e.g. via host-side `apxm backend add` with
# APXM_HOME=/var/lib/apxm/tenant-a, then copy the file in.
docker run --rm \
    -p 18800:18800 \
    -v /var/lib/apxm/tenant-a:/apxm-home:ro \
    apxm-server:latest
```

## Run two tenants

```bash
docker compose -f deploy/apxm-server/docker-compose.example.yml up
```

This brings up `apxm-tenant-a` on host port 18800 and `apxm-tenant-b`
on host port 18801, each with its own backend roster.

## Why this exists — the multi-instance contract

`BackendStore::open()` (in `crates/runtime/apxm-credentials/src/backend.rs`)
used to call `dirs::home_dir().join(".apxm")` unconditionally. That
hardcoded path made two `apxm-server` processes on the same host
share `~/.apxm/config.toml`, so they could not own distinct backend
rosters.

That code now routes through `apxm_core::env::apxm_home()`, which
honors the `APXM_HOME` environment variable and falls back to
`~/.apxm`. One `apxm-server` per `APXM_HOME` is the supported
multi-tenant deployment shape — containerized (one volume per
container, as above), systemd-instanced, or otherwise process-isolated.

The image bakes `APXM_HOME=/apxm-home` and `APXM_SERVER_ADDR=0.0.0.0:18800`
so the contract works with nothing more than a volume mount and a port
publish. Mount read-only: credential rotation belongs to the
out-of-band config-management path, not to in-container CLI calls.

## Caveats

- **Image size**: ~2.5 GB. The compiler crate (`apxm-compiler`) is a
  hard dependency of `apxm-server`, and the conda-forge MLIR 22 runtime
  libraries it links against dominate the runtime stage. A
  `server-runtime-only` Cargo feature that excludes the compile/execute
  routes (`apxm_compiler`-dependent modules in `app.rs`, `execute.rs`,
  `a2a.rs`, `mcp_tools.rs`, `bin/apxm_mcp.rs`) would let us drop the
  MLIR runtime entirely and target a much slimmer image; tracked as
  follow-up. Until then, the heavy image is the only supported variant.
- **Build host disk**: the builder stage materializes ~30 GB of
  intermediate cargo artifacts. If your Docker engine's working
  volume is tight on space, rustc fails opaquely with "No space left
  on device" during the final link (cargo reports it as
  `exit status: 101` / "failed to parse process output"). Reclaim
  with `docker builder prune -af` and rebuild.
- **glibc pinning**: builder and runtime stages both run Ubuntu 24.04
  (glibc 2.39). Swapping the runtime base to anything older (e.g.
  `debian:bookworm-slim` with glibc 2.36) fails at startup with
  "GLIBC_2.39 not found".
- **LLVM 22 distribution**: no Debian/Ubuntu apt repository ships LLVM
  22 today, which is why the build stage uses conda-forge (mirroring
  the `.dekk.toml` `[environment]` block). If the conda-forge MLIR 22
  feed disappears, the image is the same risk that `dekk apxm doctor`
  surfaces on developer machines.
- **Authority CLI**: developers should still iterate via `dekk apxm`,
  not this image. The container is for deployment, not local
  iteration.
