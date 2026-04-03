# apxm-cli

Minimal CLI wrapper for compile/run workflows.

## Overview

`apxm-cli` is a thin wrapper around `apxm-driver` that exposes:
- `compile` -- compile ApxmGraph (.apxm) to an artifact
- `execute` -- compile + execute a graph via the runtime
- `run` -- execute a precompiled artifact
- `doctor` -- verify environment, dependencies, and toolchain (powered by [dekk](https://github.com/randreshg/dekk))
- `activate` -- print shell exports for MLIR/LLVM env setup
- `install` -- create/update the conda env from environment.yaml

## Responsibilities

- Provide a minimal, scriptable CLI for compile/run workflows
- Surface environment and toolchain diagnostics (`doctor`) using dekk for platform, dependency, conda, and CI detection
- Bootstrap the MLIR toolchain with `install` + `activate`

## How It Fits

`apxm-cli` wraps `apxm-driver` for compile/run (via the `driver` feature) and
uses dekk for environment detection:
- `dekk.PlatformDetector` -- OS, arch, distro, WSL, container detection
- `dekk.DependencyChecker` -- tool existence and version validation
- `dekk.CondaDetector` -- conda/mamba environment detection
- `dekk.CIDetector` -- CI/CD provider detection and metadata extraction

The Python CLI layer (`tools/scripts/`) wraps dekk types with APXM-specific logic:
- `tools/scripts/config.py` -- `PlatformConfig` wrapping `dekk.PlatformInfo`
- `tools/scripts/deps.py` -- APXM dependency specs using `dekk.DependencySpec`
- `tools/scripts/doctor.py` -- doctor command using all dekk detectors
- `tools/scripts/ci_env.py` -- CI build settings derived from `dekk.CIDetector`

## Usage

### CLI (Recommended)

Add `apxm` to your PATH and use the CLI wrapper:

```bash
# Add to PATH (add to ~/.zshrc or ~/.bashrc for persistence)
export PATH="$PATH:$(pwd)/bin"
python -m pip install dekk

# Use the CLI
apxm doctor
apxm compiler build
apxm compiler run examples/hello_graph.apxm
```

See `docs/AGENTS.md` for the complete CLI reference.

### Binary Usage (After Building)

```bash
# Build the compiler
cargo build -p apxm-cli --features driver --release

# Use the compiled binary
./target/release/apxm execute examples/hello_graph.apxm
./target/release/apxm compile examples/hello_graph.apxm -o output.apxmobj
./target/release/apxm doctor
```

### Environment Setup Commands

```bash
# Install/update conda environment
cargo run -p apxm-cli -- install

# Emit exports for your shell
eval "$(cargo run -p apxm-cli -- activate)"
eval "$(cargo run -p apxm-cli -- activate --shell fish)"
```

## Metrics (Optional)

```bash
cargo run -p apxm-cli --features metrics -- execute examples/hello_graph.apxm
```

## Testing

```bash
cargo test -p apxm-cli
```
