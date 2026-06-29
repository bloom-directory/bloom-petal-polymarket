# Next Steps

This branch establishes the route-file build system and starts the migration
toward small, focused WASM components. It is intentionally a midpoint: all 67
Bloom route files now build independently, and the low-risk public routes use
focused route macros, but write-heavy account/onboard/fund/trade routes still
use the compatibility adapter.

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
  - the transitional `bloom_route_component!` compatibility adapter
- 20 route files have moved to focused framework macros.
- 47 route files still use the compatibility adapter.

## Size Snapshot

After `scripts/build.sh`:

- `app/polymarket/$index.wasm`: about 59 KB
- `app/polymarket/markets/[slug]/book.json.wasm`: about 500 KB
- `app/polymarket/positions/[wallet]/positions.json.wasm`: about 469 KB
- `app/polymarket/trade/[wallet]/drafts/[id]/post.wasm`: about 1.4 MB

The large trade file is still adapter-backed. The smaller public files show the
route-specific approach is paying off.

## Suggested Handoff Order

1. Move shared framework helpers out of `route/src/lib.rs` into route-local
   modules, for example `route/src/framework.rs`, `route/src/http.rs`,
   `route/src/store.rs`, and route-domain modules.
2. Convert `account` routes next. They are mostly read-only and should be less
   risky than onboarding, funding, or trade posting.
3. Convert `onboard` status/plan/approvals reads separately from
   `onboard/[wallet]/begin`, because `begin` is side-effecting and signing
   sensitive.
4. Convert `fund` read routes before `fund/[wallet]/new`.
5. Convert `trade` read routes before `revalidate`, `post`, and `cancel`.
6. Once no route file uses `bloom_route_component!`, remove the central
   `lookup`, `list`, `read`, `write`, and `path_kind` dispatcher.
7. Move the pieces currently imported from `crates/bloom-polymarket` into
   route-local modules, then remove the path dependency.
8. Re-run:

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
