#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
PETAL_REV="3e7ac46fcce2191aac6b0e41414d0eed9ec331b4"

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
