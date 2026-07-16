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

The route tree currently builds 94 route components. Directory endpoints use
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
