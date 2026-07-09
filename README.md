# Polymarket v2 Petal

This package implements the Polymarket app at `apps/polymarket/...` using the
`bloom:route@0.1.0` component ABI.

The route component source lives in `route/`. It performs
Polymarket HTTP calls directly through the v2 HTTP import, persists petal-owned
state through the v2 private store import, uses v2 signing intents for CLOB and
relayer signatures, reads mediated chain state through the generic chain
interface, and stages funding through the generic EVM outbox. Enso credentials
are provisioned through the write-only `settings/enso-api-key` route and remain
in the Petal secret store. It uses only generic v2 interfaces and does not
delegate to the legacy native `polymarket/...` VFS handler.

Generate the route components with:

```sh
scripts/build.sh
```

The generated `app/polymarket/**/*.wasm` files are ignored build output and
should not be committed.
