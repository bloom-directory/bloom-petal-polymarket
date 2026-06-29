# Next Steps

This branch establishes the route-file build system and moves the petal to
small, focused WASM components. All 48 Bloom route files build independently and
use focused route macros. Directory routes expose `list` directly through the
same component used for lookup/metadata, so separate `$list.rs` source files are
no longer needed.

## Current State

- `scripts/build.sh` delegates to `xtask`.
- `xtask` discovers `route/files/**/*.rs`, compiles each file with
  `BLOOM_ROUTE_RS`, converts it with `wasm-tools component new`, and validates
  the resulting component.
- `route/src/lib.rs` provides the selected-route include point and a small
  route framework:
  - `bloom_dir_component!`
  - `bloom_fallible_dir_component!`
  - `bloom_ctx_dir_component!`
  - `bloom_read_component!`
- `bloom_write_component!`
- `xtask` derives `BLOOM_ROUTE_PATH` and `BLOOM_ROUTE_CANONICAL_PATH` from each
  source file, so route files no longer repeat their own path strings.
- `$list.rs` sources are rejected by `xtask`; directory endpoints are `$index.rs`
  files that implement both lookup/metadata and list exports.
- The old central `lookup`/`list`/`read`/`write`/`path_kind` dispatcher has been
  removed.
- The route crate no longer depends on `crates/bloom-polymarket`; the helper
  types and signing/order modules it needs are owned by `route/src`.

## Size Snapshot

After `scripts/build.sh`:

- `app/polymarket/$index.wasm`: about 59 KB
- `app/polymarket/markets/[slug]/book.json.wasm`: about 500 KB
- `app/polymarket/positions/[wallet]/positions.json.wasm`: about 469 KB
- `app/polymarket/account/[wallet]/portfolio.json.wasm`: about 554 KB
- `app/polymarket/fund/[wallet]/new.wasm`: about 236 KB
- `app/polymarket/onboard/[wallet]/begin.wasm`: about 711 KB
- `app/polymarket/trade/[wallet]/drafts/[id]/post.wasm`: about 1.1 MB
- 44 of 48 generated components are under 700 KB; 2 remain over 1 MB.
- write-heavy trade components are still larger than public read components
  because they pull in signing, policy, CLOB posting, relayer, and receipt
  helpers directly.

## Suggested Handoff Order

1. Move shared framework helpers out of `route/src/lib.rs` into smaller
   modules, for example `route/src/framework.rs`, `route/src/http.rs`,
   `route/src/store.rs`, and route-domain modules.
2. Split the remaining route-domain helpers into focused modules, for example account,
   onboard, fund, trade reads, trade writes, relayer, and CLOB signing.
3. Measure generated component sizes again after module splitting and identify
   the largest remaining imports for each write-heavy route.
4. Re-run:

```sh
cargo test --manifest-path route/Cargo.toml
scripts/build.sh
```

Optionally run `scripts/validate.sh` when the Bloom CLI or `BLOOM_REPO` is
available. This branch assumes the Bloom runtime no longer requires separate
`$list.wasm` artifacts and will call the directory component's `list` export.

## WAT Experiment

`docs/examples/minimal-route-wat.md` and
`docs/examples/minimal-route-core.wat` document why handwritten WAT is not the
right implementation path for real route files. The component ABI requires
canonical ABI lowering/lifting for records, variants, lists, strings, results,
allocation, and cleanup.
