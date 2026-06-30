# Next Steps

This branch now uses a local `petal` crate for reusable Bloom route framework
and host SDK code. The Polymarket route crate consumes that framework through
`petal::route_file!`, `petal::Ctx`, route specs, route helpers, and
`petal::sdk`.

## Current State

- `scripts/build.sh` delegates to `xtask`.
- `xtask` discovers `route/files/**/*.rs`, compiles each route file with
  `BLOOM_ROUTE_RS`, converts it with `wasm-tools component new`, and validates
  the resulting component.
- The route tree has 47 source files after removing the old `meta` route.
- `$list.rs` sources are rejected by `xtask`; directory endpoints are `$index.rs`
  files that implement lookup, metadata, and list exports.
- `petal/` owns the Bloom route WIT, generic route macros/specs, metadata,
  param lookup, JSON response helper, error conversion, and SDK wrappers.
- `route/src/lib.rs` is now wiring: selected route include, Polymarket modules,
  and the final `petal::export!(Route)`.
- `route/src/polymarket/order_model.rs` owns reusable order draft/receipt
  models. The old filesystem `order_store.rs` is compiled only for tests.
- `route/src/onboarding/` is split into top-level begin flow, geoblock, relayer
  flow, persistence, and status/readiness modules.

## Handoff Items

1. Measure generated component sizes again after this refactor and identify
   whether any write-heavy route still pulls in avoidable modules.
2. Continue shrinking `route/src/lib.rs` re-exports by moving route files and
   domain modules toward explicit imports where practical.
3. Decide whether the test-only native `order_store.rs` coverage is still worth
   keeping in this petal repo, or whether those tests should move with any
   future native Polymarket package.
4. Re-run:

```sh
cargo test --manifest-path route/Cargo.toml
scripts/build.sh
```

Optionally run `scripts/validate.sh` when the Bloom CLI or `BLOOM_REPO` is
available.
