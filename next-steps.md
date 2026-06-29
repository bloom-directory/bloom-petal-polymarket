# Next Steps

This branch establishes the route-file build system and starts the migration
toward small, focused WASM components. All 67 Bloom route files now build
independently and use focused route macros. The old central dispatcher remains
in `route/src/lib.rs` only as dead transitional code waiting to be deleted once
shared helpers are split into modules.

## Current State

- `scripts/build.sh` delegates to `xtask`.
- `xtask` discovers `route/files/**/*.rs`, compiles each file with
  `BLOOM_ROUTE_RS`, converts it with `wasm-tools component new`, and validates
  the resulting component.
- `route/src/lib.rs` provides the selected-route include point and a small
  route framework:
  - `bloom_dir_component!`
  - `bloom_fallible_dir_component!`
  - `bloom_read_component!`
- write-capable route files now use `bloom_write_component!`
- no route files use the transitional `bloom_route_component!` compatibility
  adapter anymore.

## Size Snapshot

After `scripts/build.sh`:

- `app/polymarket/$index.wasm`: about 59 KB
- `app/polymarket/markets/[slug]/book.json.wasm`: about 500 KB
- `app/polymarket/positions/[wallet]/positions.json.wasm`: about 469 KB
- `app/polymarket/account/[wallet]/portfolio.json.wasm`: about 554 KB
- `app/polymarket/fund/[wallet]/new.wasm`: about 236 KB
- `app/polymarket/onboard/[wallet]/begin.wasm`: about 711 KB
- `app/polymarket/trade/[wallet]/drafts/[id]/post.wasm`: about 1.1 MB
- 63 of 67 generated components are under 700 KB; 2 remain over 1 MB.
- write-heavy trade components are still larger than public read components
  because they pull in signing, policy, CLOB posting, relayer, and receipt
  helpers directly.

## Suggested Handoff Order

1. Move shared framework helpers out of `route/src/lib.rs` into route-local
   modules, for example `route/src/framework.rs`, `route/src/http.rs`,
   `route/src/store.rs`, and route-domain modules.
2. Split route-domain helpers into focused modules, for example account,
   onboard, fund, trade reads, trade writes, relayer, and CLOB signing.
3. Remove the now-unused `bloom_route_component!` compatibility macro.
4. Remove the central
   `lookup`, `list`, `read`, `write`, and `path_kind` dispatcher.
5. Move the pieces currently imported from `crates/bloom-polymarket` into
   route-local modules, then remove the path dependency.
6. Re-run:

```sh
cargo test --manifest-path route/Cargo.toml
scripts/build.sh
```

Optionally run `scripts/validate.sh` when the Bloom CLI or `BLOOM_REPO` is
available.

## WAT Experiment

`docs/examples/minimal-route-wat.md` and
`docs/examples/minimal-route-core.wat` document why handwritten WAT is not the
right implementation path for real route files. The component ABI requires
canonical ABI lowering/lifting for records, variants, lists, strings, results,
allocation, and cleanup.
