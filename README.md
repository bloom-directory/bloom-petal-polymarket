# Polymarket Petal

This package implements the Polymarket Petal at `petals/polymarket/...` using the
`bloom:route@0.1.0` component ABI.

## Layout

- `petal-build.toml` pins the canonical Bloom Petal SDK and configures the
  canonical route builder.
- `route/files/` contains the route controllers. Each `*.rs` file maps to one
  Bloom route component and is compiled by the canonical `petal` CLI.
- `route/src/` contains Polymarket-specific domain code used by those route
  controllers: HTTP/store/signing infrastructure, onboarding, funding, trading,
  policy, order construction, EIP-712, wallet calls, and typed DTOs.
- `scripts/build.sh` resolves the exact canonical builder revision and installs
  the complete generated route tree only after every component succeeds.

The route tree currently builds 95 route components. Directory endpoints use
`$index.rs`; `$list.rs` is intentionally unsupported because the Bloom Guest
world already has a separate `list` export.

## Runtime Boundaries

This petal owns the Polymarket behavior. It performs Polymarket HTTP calls
directly through the HTTP import, persists petal-owned state through the
private store import, uses signing intents for CLOB and relayer signatures,
reads mediated wallet/chain state through generic Bloom interfaces, and stages
funding through the generic EVM outbox. Enso credentials are provisioned
through the write-only `settings/enso-api-key` route and remain in the Petal
secret store.

It uses only the Petal route ABI and does not delegate to the legacy native
`polymarket/...` VFS handler.

Generated `petal/polymarket/**/*.wasm` files are ignored build output and should
not be committed.

Generate the route components with:

```sh
scripts/build.sh
```

Run tests with:

```sh
cargo test --manifest-path route/Cargo.toml
```

After changing `route/files/`, `route/src/`, or `petal-build.toml`, run both
commands before opening or updating a PR.

## Releases

Installable Petal archives are built and published by the reusable release
workflow in [`bloom-directory/petal`](https://github.com/bloom-directory/petal).
This repository owns only the thin tag-triggered caller in
`.github/workflows/release.yml`; it must pin that workflow to a full commit SHA.
The caller also passes that exact commit as `petal-tooling-ref`, so the workflow
implementation and the CLI that creates the package come from one reviewed
revision.

Release tags use Semantic Versioning with a `v` prefix. The first release that
publishes an installable archive is `v0.1.3`; the existing `v0.1.0` through
`v0.1.2` source tags remain unchanged. For a tag such as `v0.1.3`, the canonical
release files are:

- `polymarket-v0.1.3.petal.tar.gz`
- `SHA256SUMS`
- `petal-release.json`

The archive is platform-neutral: it contains WebAssembly route components and
Petal package metadata, not host-native executables. Bloom pins a release tag,
its resolved source commit, the asset filename, and its checksum; setup then
downloads and validates that immutable asset from this repository instead of
compiling the Petal locally.

To publish a release:

1. Ensure CI passes on `main` and choose the next Semantic Versioning tag.
2. Create the tag from the exact reviewed commit and push it.
3. Confirm the release workflow publishes all three files and that the source
   commit in `petal-release.json` matches the tagged commit.
4. Update Bloom's built-in Petal catalog in a separate reviewed change.

Do not retag or replace an existing release asset. Publish a new patch release
for packaging-only corrections.
