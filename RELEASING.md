# Releasing apxm

This doc covers releases of the `apxm` Python package on PyPI, plus the two
npm packages built out of this repo (`@apxm/frontend`, `@apxm/client`; see
[npm packages](#npm-packages-apxmfrontend-apxmclient) below). The Rust
crates are not yet published — the workspace tracks the release version and waits
for the public Rust API to stabilize.

PyPI releases are cut **manually** from a maintainer's machine through
`dekk agents release`. There is intentionally no GitHub Actions workflow that
auto-publishes the Python package: the maintainer's PyPI API token never
leaves the local environment, and every release is a deliberate human
action. The npm packages are the exception — see below — because a package
is unregistered until its LICENSE and compiled-artifact-only layout is
verified, so tag-gated CI publish carries less risk than the PyPI flow.

## Versioning

- **SemVer**, bumped in `crates/compiler/frontend/python/pyproject.toml`.
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
dekk agents release check
```

## Cutting a release

1. Bump `version =` in `crates/compiler/frontend/python/pyproject.toml`
   and `[workspace.package].version` in `Cargo.toml`.
2. Commit:
   `chore(release): bump apxm to v0.X.Y`
3. Open and merge the PR.
4. Tag the release on `main`:
   ```bash
   git tag -a v0.X.Y -m "apxm 0.X.Y"
   git push origin v0.X.Y
   ```
5. From a clean checkout of the tagged commit, rebuild the artifacts:
   ```bash
   dekk agents release dist
   ```
   Artifacts are written under `.apxm/releases/v0.X.Y/`, including the
   Python wheel/sdist, binary archive, source archive, and `SHA256SUMS`.
6. Upload to PyPI when the Python package is ready:
   ```bash
   dekk agents release pypi --yes
   ```
   `twine` reads credentials from `~/.pypirc` (or
   `TWINE_USERNAME`/`TWINE_PASSWORD` env vars). Confirm the upload by
   visiting <https://pypi.org/project/apxm/0.X.Y/>.
7. Create the GitHub release:
   ```bash
   dekk agents release publish --yes
   ```

## Test publishing (optional)

To rehearse a release against TestPyPI:

```bash
dekk agents release pypi --repository testpypi --yes
```

Requires a separate TestPyPI account + token under a `[testpypi]` block
in `~/.pypirc`. Install from TestPyPI to verify:

```bash
pip install --index-url https://test.pypi.org/simple/ apxm==0.X.Y
```

## Yanking a release

If a release ships a regression that downstream `eval`
consumers depend on, **yank** rather than delete. Yanked
releases stay installable for pinned users but are skipped by
`pip install apxm`:

1. PyPI UI: project → release → "Options" → "Yank".
2. Ship the fix on the next patch version.

Do not attempt to re-upload the same version — PyPI rejects overwrites
even after a yank.

## npm packages (`@apxm/frontend`, `@apxm/client`)

Per masterplan decision D6, packages built out of this private repo publish
to npm as **Apache-2.0-licensed, compiled-artifact-only** packages (`dist/`
+ typings + `LICENSE`, no duplicated `src/`) even though the `agents` repo
itself stays MIT. This applies to:

- `@apxm/frontend` (`crates/compiler/frontend/typescript/`)
- `@apxm/client` (`crates/tools/client/typescript/`)

Unlike the PyPI flow above, npm releases run through
`.github/workflows/release.yml` and are **tag-triggered**:

1. Bump `version` in the package's `package.json`.
2. Commit and merge as usual.
3. Tag the release commit with the package-scoped prefix:
   ```bash
   git tag -a frontend-v0.X.Y -m "@apxm/frontend 0.X.Y"
   git push origin frontend-v0.X.Y
   # or
   git tag -a client-v0.X.Y -m "@apxm/client 0.X.Y"
   git push origin client-v0.X.Y
   ```
4. The matching CI job (`frontend` or `client`) installs, typechecks, tests,
   builds, then runs `npm publish --access public` using `NPM_TOKEN` from
   repo secrets. Confirm at
   `https://www.npmjs.com/package/@apxm/frontend` or
   `https://www.npmjs.com/package/@apxm/client`.

To rehearse a publish without a tag push (e.g. to confirm the tarball
contents before cutting a release), use the workflow's manual dispatch path,
which runs `npm publish --dry-run --access public` instead of a real
publish:

```bash
gh workflow run release.yml -f target=frontend
gh workflow run release.yml -f target=client
```

You can also rehearse locally from the package directory:

```bash
npm run build && npm run typecheck && npm run test
npm pack --dry-run
```
