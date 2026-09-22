#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
PETAL_REV="2beed2ff344ce2b0c112e07096027e1ae0404007"

if [[ "${PETAL_COMPILE_TIME_SECRET+x}" == "x" ]]; then
  if [[ -z "$PETAL_COMPILE_TIME_SECRET" ]]; then
    echo "PETAL_COMPILE_TIME_SECRET is not configured" >&2
    exit 1
  fi
  export POLYMARKET_BUILDER_CODE="$PETAL_COMPILE_TIME_SECRET"
  unset PETAL_COMPILE_TIME_SECRET
fi

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
