# Minimal route WAT experiment

This is a deliberately tiny, non-functional route component skeleton for the
`bloom:route/route-file@0.1.0` world.

Generate the core WAT skeleton:

```sh
wasm-tools component embed route/wit --world route-file --dummy -t \
  -o docs/examples/minimal-route-core.wat
```

Convert it to a component:

```sh
wasm-tools component new docs/examples/minimal-route-core.wat \
  -o /private/tmp/minimal-route-component.wasm
```

Validate it:

```sh
wasm-tools validate /private/tmp/minimal-route-component.wasm
```

The generated core module exports the lowered component ABI functions:

```wat
(export "cm32p2||metadata" (func ...))
(export "cm32p2||lookup" (func ...))
(export "cm32p2||list" (func ...))
(export "cm32p2||read" (func ...))
(export "cm32p2||write" (func ...))
(export "cm32p2_memory" (memory ...))
(export "cm32p2_realloc" (func ...))
```

All route functions currently trap with `unreachable`. This proves the minimum
component shape, but it is not a usable Bloom route file.

Making it useful by hand means implementing the component model canonical ABI:
records, variants, results, strings, lists, allocation, and post-return cleanup.
That is why handwritten WAT is only practical for ABI experiments, not for real
Polymarket routes.
