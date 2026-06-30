# Polymarket v2 Petal

- Keep this package on `bloom:route@0.1.0`; do not add legacy or compat route
  artifacts.
- The route component owns the Polymarket behavior. It may use v2 HTTP, store,
  signing, and mediated wallet/chain VFS imports, but it must not delegate to
  the legacy native `polymarket/...` VFS handler.
- After changing `route/src/lib.rs` or the route list, run
  `scripts/build.sh` to regenerate local route
  artifacts for validation. Do not commit generated `.wasm` artifacts.

## Route/controller/service shape

- Treat `route/files/**/*.rs` as the route controllers. A route file should own
  the endpoint-level composition: route params, list/read/write selection, small
  response assembly, and the call into lower-level helpers or services.
- Do not hide simple endpoint reads behind one-line service wrappers. If an
  endpoint just reads a stored artifact, loads one object, or renders one small
  response, keep that composition in the route file.
- Put reusable business workflows in `route/src/services/`: creating drafts,
  revalidating/posting/cancelling trades, onboarding, funding, relayer flows,
  policy evaluation, HTTP/store/chain orchestration, and other multi-step
  behavior.
- Keep framework code generic in `route/src/framework.rs`: route specs, entry
  metadata, param lookup, route macros, and response/error conversion. Framework
  helpers must not know Polymarket route semantics.
- Keep host import wrappers in `route/src/host.rs`; route files and services
  should use the local wrappers instead of WIT bindings directly.
- Keep Polymarket protocol/domain helpers in `route/src/polymarket/`: order
  construction, signing, typed API structs, wallet calls, EIP-712, and credential
  types.
- Avoid stringly typed dispatch. Do not add generic helpers like
  `read_trade(kind, file)`, `read_fund(file)`, `read_market(file)`, or service
  facades that only pass route/file names into a central dispatcher.
- Do not pass arbitrary context labels such as `"trade plan"` through helper
  APIs. Error/context text should be owned by the function that creates the
  error.
- File names may appear at the storage/path boundary, but prefer named constants,
  typed enums, or explicit key-builder helpers when the same file name is reused.
- Do not introduce route-param convenience wrappers unless they remove real
  duplication across complex controllers. Straightforward `crate::param(ctx,
  "...")` extraction in a route file is fine.
- A good service boundary is: controllers compose endpoint-specific behavior;
  services implement reusable workflows; storage/path helpers build keys and
  perform low-level reads/writes; Polymarket modules implement protocol details.
