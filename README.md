# Polymarket v2 Petal

This package implements the Polymarket app at `apps/polymarket/...` using the
`bloom:route@0.1.0` component ABI.

## Layout

- `petal/` contains the local Bloom petal framework crate: route WIT bindings,
  route macros/specs, metadata and param helpers, response/error conversion, and
  host SDK wrappers.
- `route/files/` contains the route controllers. Each `*.rs` file maps to one
  Bloom route component and is compiled independently by `xtask`.
- `route/src/` contains Polymarket-specific domain code used by those route
  controllers: HTTP/store/signing infrastructure, onboarding, funding, trading,
  policy, order construction, EIP-712, wallet calls, and typed DTOs.
- `xtask/` discovers route files, derives route metadata from file paths, builds
  each selected route, converts it with `wasm-tools component new`, and
  validates the result.

The route tree currently builds 47 route components. Directory endpoints use
`$index.rs`; `$list.rs` is intentionally unsupported because the Bloom Guest
world already has a separate `list` export.

## Runtime Boundaries

This petal owns the Polymarket behavior. It performs Polymarket HTTP calls
directly through the v2 HTTP import, persists petal-owned state through the v2
private store import, uses v2 signing intents for CLOB and relayer signatures,
and reads mediated wallet/chain state through the host VFS imports.

It uses only the v2 route ABI and does not delegate to the legacy native
`polymarket/...` VFS handler.

Generated `app/polymarket/**/*.wasm` files are ignored build output and should
not be committed.

Generate the route components with:

```sh
scripts/build.sh
```

Run tests with:

```sh
cargo test --manifest-path route/Cargo.toml
```

After changing `petal/`, `route/files/`, `route/src/`, or `xtask/`, run both
commands before opening or updating a PR.
