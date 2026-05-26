# Releasing apxm

This doc covers releases of the `apxm` Python package on PyPI. The Rust
crates are not yet published — the workspace tracks `0.0.1` and waits
for the public Rust API to stabilize (see Phase R.10.5 in
`docs/design/apxm-skill-runtime-task-backlog.md`).

Releases are cut **manually** from a maintainer's machine using `twine`.
There is intentionally no GitHub Actions workflow that auto-publishes:
the maintainer's PyPI API token never leaves the local environment, and
every release is a deliberate human action.

## Versioning

- **SemVer**, bumped in `crates/compiler/apxm-frontend/python/pyproject.toml`.
- `0.x.y` while the AIS dialect is unstable.
- Bump the **minor** for any AIS op change that consumers might depend
  on (op name, input/output shape, attribute taxonomy).
- Bump the **patch** for fixes that don't change the IR surface.
- Bump the **major** to `1.0.0` only when AIS is declared stable and
  the dialect is committed to backwards-compatibility guarantees.

## One-time setup

1. Claim the `apxm` project name on PyPI (first publish creates it; the
   maintainer who publishes `0.1.0` becomes the project owner).
2. Generate a **project-scoped** API token on PyPI:
   - Account settings → API tokens → Add API token
   - Scope: *Project: `apxm`* (NOT account-wide)
3. Store the token in `~/.pypirc` (chmod `0600`) — never in this repo:
   ```ini
   [pypi]
   username = __token__
   password = pypi-AgENdGVzdC5weXBpLm9yZwIk...
   ```
   Alternatively pass `TWINE_USERNAME=__token__` and
   `TWINE_PASSWORD=<token>` via the environment at upload time.

## Pre-release checks

```bash
# Generated files are current
dekk apxm codegen
git diff --exit-code crates/compiler/apxm-frontend/python/apxm/_generated/

# Tests pass
dekk apxm test-python-frontend

# Build cleanly in a fresh venv
python3 -m venv /tmp/apxm-build && /tmp/apxm-build/bin/pip install build
/tmp/apxm-build/bin/python -m build crates/compiler/apxm-frontend/python/
```

Verify the wheel installs and imports in a clean venv:

```bash
python3 -m venv /tmp/apxm-smoke
/tmp/apxm-smoke/bin/pip install crates/compiler/apxm-frontend/python/dist/apxm-*.whl
/tmp/apxm-smoke/bin/python -c "from apxm.contract import RepoLayout, build_layout; from apxm import GraphRecorder, compile; print('OK')"
```

## Cutting a release

1. Bump `version =` in `crates/compiler/apxm-frontend/python/pyproject.toml`.
2. Commit:
   `chore(release): bump apxm python to v0.X.Y`
3. Open and merge the PR.
4. Tag the release on `main`:
   ```bash
   git tag -a v0.X.Y -m "apxm 0.X.Y"
   git push origin v0.X.Y
   ```
5. From a clean checkout of the tagged commit, rebuild the artifacts:
   ```bash
   rm -rf crates/compiler/apxm-frontend/python/dist/
   python3 -m venv /tmp/apxm-release && /tmp/apxm-release/bin/pip install build twine
   /tmp/apxm-release/bin/python -m build crates/compiler/apxm-frontend/python/
   /tmp/apxm-release/bin/python -m twine check crates/compiler/apxm-frontend/python/dist/*
   ```
6. Upload to PyPI:
   ```bash
   /tmp/apxm-release/bin/python -m twine upload \
     crates/compiler/apxm-frontend/python/dist/apxm-0.X.Y*
   ```
   `twine` reads credentials from `~/.pypirc` (or
   `TWINE_USERNAME`/`TWINE_PASSWORD` env vars). Confirm the upload by
   visiting <https://pypi.org/project/apxm/0.X.Y/>.
7. Create a GitHub release for the tag (release notes only, no asset
   upload — PyPI is the artifact store):
   ```bash
   gh release create v0.X.Y \
     --title "apxm 0.X.Y" \
     --notes-file release-notes/v0.X.Y.md
   ```

## Test publishing (optional)

To rehearse a release against TestPyPI:

```bash
/tmp/apxm-release/bin/python -m twine upload \
  --repository testpypi \
  crates/compiler/apxm-frontend/python/dist/apxm-0.X.Y*
```

Requires a separate TestPyPI account + token under a `[testpypi]` block
in `~/.pypirc`. Install from TestPyPI to verify:

```bash
pip install --index-url https://test.pypi.org/simple/ apxm==0.X.Y
```

## Yanking a release

If a release ships a regression that downstream `apxm-eval` or
`apxm-libs` consumers depend on, **yank** rather than delete. Yanked
releases stay installable for pinned users but are skipped by
`pip install apxm`:

1. PyPI UI: project → release → "Options" → "Yank".
2. Ship the fix on the next patch version.

Do not attempt to re-upload the same version — PyPI rejects overwrites
even after a yank.
