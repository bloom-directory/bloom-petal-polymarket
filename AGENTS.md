# Polymarket v2 Petal

- Keep this package on `bloom:route@0.1.0`; do not add legacy or compat route
  artifacts.
- The route component owns the Polymarket behavior. It may use v2 HTTP, store,
  signing, and mediated wallet/chain VFS imports, but it must not delegate to
  the legacy native `polymarket/...` VFS handler.
- After changing `route/src/lib.rs` or the route list, run
  `scripts/build.sh` to regenerate local route
  artifacts for validation. Do not commit generated `.wasm` artifacts.

## Route/controller/module shape

- Treat `route/files/**/*.rs` as the route controllers. A route file should own
  the endpoint-level composition: route params, list/read/write selection, small
  response assembly, and any simple endpoint behavior.
- There is no service layer. Do not add `route/src/services/`, `route/src/app.rs`,
  or another catch-all module that centralizes route behavior under a new name.
- Keep simple business behavior in the route file: static hints, static child
  lists, one-off store keys, direct store reads, small JSON/text responses, and
  straightforward calls to Bloom host wrappers.
- Route-local facts belong in the route file. The exact backing store key for an
  endpoint, static read hint, static child list, and one-off
  `read_store(format!(...))` call should be visible in `route/files/**`, not
  hidden behind a shared helper whose name mirrors the route.
- Add a Rust module only when it contains substantial reusable logic or a clear
  domain boundary. Good module candidates are protocol code, typed DTOs, policy
  evaluation, trade/fund/onboarding workflows, relayer orchestration,
  HTTP/store/signing infrastructure, and other multi-step behavior reused by
  more than one route.
- Keep framework code generic in `route/src/framework.rs`: route specs, entry
  metadata, param lookup, route macros, and response/error conversion. Framework
  helpers must not know Polymarket route semantics.
- Keep host import wrappers in `route/src/host.rs`; route files and domain
  modules should use the local wrappers instead of WIT bindings directly.
- Keep Polymarket protocol/domain helpers in `route/src/polymarket/`: order
  construction, signing, typed API structs, wallet calls, EIP-712, and credential
  types.
- Avoid stringly typed dispatch. Do not add generic helpers like
  `read_trade(kind, file)`, `read_fund(file)`, `read_market(file)`, or facades
  that only pass route/file names into a central dispatcher.
- Do not add route-facing helpers that only format endpoint paths or wrap static
  responses. Avoid helpers such as `trade_draft_policy_check_key(wallet, id)`,
  `trade_post_hint()`, or `account_orders_json(wallet)` when the route can
  directly express the path, hint, or small response.
- Do not extract single-endpoint render helpers. If only
  `route/files/trade/[wallet]/drafts/[id]/plan.md.rs` needs a Markdown body,
  the `format!(...)` for that body belongs in that route file, not in
  `render_trade_plan(draft)`. Extract rendering only when multiple routes share
  the same non-trivial representation or the renderer is a domain formatter with
  independent tests.
- Do not pass arbitrary context labels such as `"trade plan"` through helper
  APIs. Error/context text should be owned by the function that creates the
  error.
- File names may appear at the storage/path boundary. Prefer inline route-local
  paths for one-off endpoint reads. Use named constants, typed enums, or
  key-builder helpers only for real shared invariants inside multi-step
  workflows, not to make route files thinner.
- Do not introduce route-param convenience wrappers unless they remove real
  duplication across complex controllers. Straightforward `crate::param(ctx,
  "...")` extraction in a route file is fine.
- A good boundary is: route controllers compose endpoint-specific behavior;
  focused modules implement reusable workflows or infrastructure; Polymarket
  modules implement protocol details. If a module starts looking like an index
  of route handlers, move that behavior back into the corresponding route files.
