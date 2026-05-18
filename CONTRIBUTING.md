# Contributing to APXM

Thanks for your interest in APXM. This guide is the short version: how to set
up a working tree, build, test, and propose a change. The longer-form theory
lives under [`docs/pxm/`](docs/pxm/) and the practical positioning lives in
[`VISION.md`](VISION.md) and the root [`README.md`](README.md).

## Getting set up

APXM is a Rust workspace with a Python frontend, an MLIR-based compiler, and a
small set of TypeScript/CSS for the GUI. The supported install path is
[Dekk](https://github.com/randreshg/dekk).

```bash
git clone https://github.com/randreshg/apxm
cd apxm
git submodule update --init --recursive    # APXM-vLLM fork under external/vllm
dekk apxm install --no-interactive
dekk apxm doctor
```

`dekk apxm install` creates a repo-local conda environment with MLIR/LLVM 22
and pinned tooling, builds the Rust workspace, and installs the Python
frontend in editable mode. After this, you should be able to run `cargo
check --workspace` and `pytest` without further setup.

## Building and testing

```bash
# Rust workspace build / lint / test
cargo check --workspace
cargo clippy --workspace --all-targets
cargo test --workspace

# Python frontend tests
pytest crates/compiler/apxm-frontend/python

# CLI smoke
dekk apxm doctor
dekk apxm ops list
```

If you change the AIS dialect, run `dekk apxm regen` (or the equivalent
codegen target) and commit the regenerated files alongside your change.

## Working on APXM-vLLM

The repo includes [`external/vllm`](external/vllm) as a git submodule pointing
at the graph-aware vLLM fork APXM uses. The supported APXM-vLLM serving path is
the Dekk-controlled Docker image path:

```bash
export APXM_VLLM_HF_HOME="$HOME/.cache/huggingface-apxm-vllm"
dekk apxm vllm doctor
dekk apxm vllm docker-build --image apxm-vllm-runtime:<TAG> --base-image <VLLM_IMAGE_TAG_OR_DIGEST>
dekk apxm vllm docker-save --image apxm-vllm-runtime:<TAG>

# Author a local zoo manifest from the template, then apply it.
cp deploy/vllm/zoo.example.toml deploy/vllm/zoo.toml  # then edit
dekk apxm vllm zoo-cache-warm                          # CPU-only HF download
dekk apxm vllm zoo-apply                               # submits Slurm jobs
dekk apxm vllm service-list                            # watch readiness
dekk apxm vllm service-exec <NAME> -- dekk apxm execute <GRAPH.py>
```

`APXM_VLLM_HF_HOME` must point at a filesystem every Slurm compute node
can read at the same path. If your `$HOME` is space-constrained, point
it at a cluster-shared mount instead — see
[`docs/backends/storage-layout.md`](docs/backends/storage-layout.md) for
the placement rules and a safe migration procedure.

The zoo manifest is the single operator surface — `service-start` and
`service-adopt` are not public CLI. See
[`docs/backends/model-zoo-quickstart.md`](docs/backends/model-zoo-quickstart.md)
for the 15-minute walkthrough,
[`docs/backends/model-zoo.md`](docs/backends/model-zoo.md) for the
reference, and [`docs/external-vllm-fork.md`](docs/external-vllm-fork.md)
for the integration contract.

## Submitting a change

1. Create a branch off `main`. Pick a short, kebab-case branch name describing
   the change (e.g. `fix-skill-server-double-escape`, `add-checkpoint-replay`).
2. Keep commits small and self-contained. The first line of a commit message
   should be ≤72 characters and use the imperative mood. Larger commits are
   fine when they belong together; squash whenever it improves bisectability.
3. Run the build and the tests above before opening a PR.
4. In the PR description, describe **what** changed and **why**. Link to the
   related issue or design doc if there is one. If your change touches a
   compiler pass, the runtime, or a public API, note the impact on existing
   skill artifacts.
5. AI/vibe-coded PRs are welcome. The same review and test bar applies.

## Reporting bugs

Open an issue at <https://github.com/randreshg/apxm/issues> with:

- The version (`dekk apxm doctor` output is helpful).
- The minimal AIR / Python frontend reproduction.
- The expected behavior, the observed behavior, and any session directory
  paths or stack traces.

For security-sensitive issues, use the contact in [`SECURITY.md`](SECURITY.md)
instead of a public issue.

## Code of conduct

This project follows the [Contributor Covenant 2.1](CODE_OF_CONDUCT.md). By
participating you agree to abide by it.

## License

APXM is MIT-licensed (see [`LICENSE`](LICENSE)). By contributing, you agree
that your contribution is licensed under the same MIT terms.
