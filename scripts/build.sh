#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
PETAL_REV="2beed2ff344ce2b0c112e07096027e1ae0404007"

# No build-time inputs: the release default builder code is declared in
# route/src/builder_code.rs, so a tagged source rebuilds byte for byte and the
# package CI checks is the package that ships.
unset PETAL_COMPILE_TIME_SECRET POLYMARKET_BUILDER_CODE

if [[ -n "${PETAL_BIN:-}" ]]; then
  "$PETAL_BIN" build --root "$ROOT"
else
  tool_root="$ROOT/target/petal-tool"
  cargo install \
    --git https://github.com/bloom-directory/petal \
    --rev "$PETAL_REV" \
    --locked \
    --root "$tool_root" \
    bloom-petal-cli
  "$tool_root/bin/petal" build --root "$ROOT"
fi
