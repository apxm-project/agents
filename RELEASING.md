# Releasing agents surfaces

All packages and release artifacts from this repository are private members of
one APXM Release Family. Publication is blocked until the release controller
supplies exact signed registry configuration and the package is a canonical
surface named by the accepted workspace architecture.

## Current publication state

- Every Rust crate has `publish = false`. There is no Cargo publication
  command in this repository.
- `@apxm/frontend` is a canonical private npm coordinate and declares
  `publishConfig.access = restricted`. No npm registry endpoint or publish
  command is configured here.
- The handwritten `@apxm/client` is prototype evidence, not a generated remote
  client, and is marked `private = true`.
- The current Python `apxm` distribution is prototype packaging and is marked
  non-publishable. Canonical Python distributions use focused private
  coordinates such as `apxm-frontend` and `apxm-compiler` after their owning
  implementation lanes land.

The coordinator owns exact registry endpoints, namespace authority, and the
Compatibility Set. This repository does not guess or default any endpoint.

## Readiness and artifacts

Release work goes through Dekk:

```bash
dekk agents release check
dekk agents release dist
dekk agents release publish
```

`release check` enumerates every tracked Cargo, npm, and Python manifest plus
every release command surface. It rejects a publishable Rust crate, public or
default npm access, an unclassified Python distribution, and public/default
registry commands.

`release dist` writes eligible artifacts and checksums under
`.apxm/releases/vX.Y.Z/`. Non-publishable Python packaging is excluded from the
release artifact set. `release publish` is a dry run unless `--yes` is passed;
the mutating path verifies that the GitHub repository visibility is exactly
`PRIVATE` before creating or updating a release.

## Private Python registry publication

The Python publication surface requires all four release-controller inputs:

- a strict `apxm.private-package-registry.v1` JSON manifest;
- its detached OpenSSH signature;
- the trusted allowed-signers file; and
- the exact signer identity.

The signed manifest contains only these fields:

```json
{
  "schema_version": "apxm.private-package-registry.v1",
  "ecosystem": "python",
  "visibility": "private",
  "repository_url": "<exact credential-free HTTPS endpoint>",
  "package_names": ["<authorized canonical distribution>"]
}
```

Do not commit the manifest, signature, allowed-signers file, registry endpoint,
or credentials. Once a canonical Python package is marked publishable and the
release controller supplies those external inputs, use:

```bash
dekk agents release python \
  --registry-manifest <path> \
  --registry-signature <path> \
  --allowed-signers <path> \
  --signer <identity>
```

The command verifies the signature and package authorization before its dry
run. `--yes` performs the upload with an explicit `--repository-url`; there is
no repository-name or implicit-index option. The command rejects known public
Python index hosts, credential-bearing URLs, unsigned configuration,
noncanonical coordinates, and packages marked non-publishable.

## Version and release sequence

1. Update the family version only through its owning release lane.
2. Add release notes for the exact version.
3. Run `dekk agents release check` and the repository gates.
4. Merge through the protected integration and promotion queues.
5. Build artifacts from the exact promoted revision.
6. Publish only to destinations admitted by the signed release inputs.

Previously published artifacts remain immutable registry history. A new
release uses a new family version; it never overwrites an existing artifact.
