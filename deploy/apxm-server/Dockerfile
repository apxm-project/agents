# syntax=docker/dockerfile:1.7
#
# apxm-server container image (multi-instance ready).
#
# The build stage uses miniforge so it can install the same conda-forge
# MLIR/LLVM 22 toolchain that .dekk.toml provisions on developer machines.
# That keeps a single source of truth for the compiler dependency surface
# and avoids hand-rolling an LLVM 22 from source (no Debian/Ubuntu apt
# repository ships LLVM 22 today). The runtime stage is a slim Debian
# image with only the compiled binary and the conda-shipped MLIR runtime
# libraries copied in.
#
# Build from the apxm repo root:
#     docker build -t apxm-server:latest -f deploy/apxm-server/Dockerfile .
#
# Expected image size: ~2.5 GB (MLIR 22 runtime libraries dominate).
# See deploy/apxm-server/README.md for the slimming follow-up.

# ── Stage 1 — builder ──────────────────────────────────────────────
# Ubuntu 24.04 base (gcc-13 / libstdc++ 13). Older focal-based
# miniforge tags ship libstdc++ 9, which is missing symbols that
# conda-forge MLIR 22's static libs expect (e.g.
# `std::__throw_bad_array_new_length` from C++17 / libstdc++ 11+) —
# linking against `lib{MLIRTransforms,MLIRBytecodeReader}.a` fails
# with "undefined reference" on focal.
FROM condaforge/miniforge3:25.3.1-0 AS builder

ARG RUST_TOOLCHAIN=nightly
ENV CARGO_TARGET_DIR=/build-target \
    DEBIAN_FRONTEND=noninteractive \
    CONDA_ENV=/opt/conda/envs/apxm

# System packages needed by the compiler's build.rs. CMake is *not*
# installed here — Ubuntu 20.04 focal (miniforge3's base) ships 3.16,
# but apxm-compiler requires 3.20+. Pick up a current cmake from
# conda-forge alongside MLIR/LLVM below. reqwest uses rustls-tls (see
# workspace Cargo.toml), so no system libssl is required.
RUN apt-get update && apt-get install -y --no-install-recommends \
        build-essential \
        curl \
        git \
        ninja-build \
        pkg-config \
    && rm -rf /var/lib/apt/lists/*

# Mirror .dekk.toml's [environment.packages] block so the container's
# MLIR toolchain matches what developers run against locally. cmake is
# added here (not via apt) because the focal-shipped 3.16 is too old.
RUN mamba create -y -n apxm -c conda-forge \
        cmake \
        mlir=22 \
        libclang13=22 \
        clang=22 \
        python=3.11 \
    && mamba clean -afy

# Install the pinned Rust nightly. rust-toolchain.toml inside the
# checkout would pick this up automatically, but installing here keeps
# the layer cacheable independently of the source tree.
RUN curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs \
        | sh -s -- -y --default-toolchain "${RUST_TOOLCHAIN}" \
            --profile minimal \
            --component rust-src
ENV PATH=/root/.cargo/bin:${CONDA_ENV}/bin:${PATH}

# Toolchain paths consumed by crates/compiler/apxm-compiler/build.rs,
# mirroring the [environment.variables] block in .dekk.toml.
ENV MLIR_DIR=${CONDA_ENV}/lib/cmake/mlir \
    LLVM_DIR=${CONDA_ENV}/lib/cmake/llvm \
    MLIR_PREFIX=${CONDA_ENV} \
    LLVM_PREFIX=${CONDA_ENV} \
    LD_LIBRARY_PATH=${CONDA_ENV}/lib

WORKDIR /src
COPY . /src

# Build only what the server needs. -p apxm-server pulls apxm-compiler
# transitively, so MLIR/LLVM 22 must be present (above). Expect ~30 GB
# of intermediate build artifacts under /build-target — make sure the
# Docker engine's working volume has enough free space, otherwise
# rustc fails with "No space left on device" during the final link.
RUN cargo build --release -p apxm-server

# ── Stage 2 — runtime ──────────────────────────────────────────────
# Must match the builder's glibc (Ubuntu 24.04 ships glibc 2.39, which
# the linked apxm-server binary depends on). debian:bookworm-slim has
# glibc 2.36 and fails with "GLIBC_2.39 not found" at startup.
FROM ubuntu:24.04 AS runtime

ENV CONDA_ENV=/opt/conda/envs/apxm \
    LD_LIBRARY_PATH=/opt/conda/envs/apxm/lib \
    APXM_HOME=/apxm-home \
    APXM_SERVER_ADDR=0.0.0.0:18800 \
    RUST_LOG=info,apxm_server=info

RUN apt-get update && apt-get install -y --no-install-recommends \
        ca-certificates \
        curl \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --create-home --shell /usr/sbin/nologin --uid 10001 apxm \
    && mkdir -p /apxm-home \
    && chown apxm:apxm /apxm-home

# Copy the MLIR/LLVM 22 shared libraries the compiler links against.
# The full conda env lib/ directory ships as a unit; trimming further
# requires a dedicated audit of the compiler crate's link surface.
COPY --from=builder /opt/conda/envs/apxm/lib /opt/conda/envs/apxm/lib

# The compiler crate emits libapxm_compiler_c.so under its cargo build
# output dir. The binary has no RPATH set, so the .so must land on
# LD_LIBRARY_PATH (already pointed at the conda lib dir above).
COPY --from=builder \
    /build-target/release/build/apxm-compiler-*/out/build/lib/libapxm_compiler_c.so \
    /opt/conda/envs/apxm/lib/

COPY --from=builder /build-target/release/apxm-server /usr/local/bin/apxm-server

USER apxm
WORKDIR /apxm-home

# $APXM_HOME is the multi-instance contract: BackendStore::open() reads
# config.toml from this directory, so each container gets its own
# backend roster by mounting a distinct host directory. See
# deploy/apxm-server/README.md.
VOLUME ["/apxm-home"]
EXPOSE 18800

ENTRYPOINT ["/usr/local/bin/apxm-server"]
